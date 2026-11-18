//! File dialogs through Android's document picker (the Storage Access Framework).
//!
//! The interface asks its host for dialogs through `Hooks` and gets back plain file paths, as on
//! the desktop. The picker gives out `content://` URIs instead, so the Java side
//! (`MainActivity.java`) turns each into a path:
//!
//! * **Open / import:** the chosen documents are copied into the app's `files/Media` folder and
//!   their paths returned. A project that refers to footage keeps working after a restart.
//! * **Save:** the person picks a name and place with the system "create document" screen; the
//!   path returned is a staging file in the app's storage, and the Java side copies it to the
//!   chosen document each time it stops changing (so a render that is still being written is
//!   copied when it is complete).
//! * **Folder:** the app's own `EffectCraft` folder in external storage (no path exists for a tree
//!   the person picks).
//!
//! The dialogs start on the Android UI thread; the answer comes back through
//! [`Java_ai_storyteller_effectcraft_MainActivity_onPicked`] on a Java thread (never the one that
//! draws), after any copying is done.

use effectcraft_ui_egui::{DialogDone, EffectcraftApp, FileDialog};
use jni::objects::{JObject, JObjectArray, JString, JValue};
use jni::sys::jint;
use jni::{JNIEnv, JavaVM};
use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Mutex, mpsc};
use std::time::Duration;
use winit::platform::android::activity::AndroidApp;

const KIND_OPEN: jint = 0;
const KIND_OPEN_PROJECT: jint = 1;
const KIND_FOLDER: jint = 2;
const KIND_SAVE: jint = 3;

/// How long a blocking hook waits for the person (they may switch apps in between).
const BLOCKING_WAIT: Duration = Duration::from_secs(30 * 60);

struct Pending {
    done: DialogDone,
    ctx: egui::Context,
}

static NEXT_ID: AtomicI32 = AtomicI32::new(1);
static PENDING: Mutex<Option<HashMap<jint, Pending>>> = Mutex::new(None);

fn pending<R>(f: impl FnOnce(&mut HashMap<jint, Pending>) -> R) -> R {
    // A poisoned lock only means another thread panicked while holding it; the map is still usable.
    let mut guard = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(HashMap::new))
}

/// Install the dialog hooks on `app`.
pub fn install(app: &mut EffectcraftApp, android: &AndroidApp, ctx: &egui::Context) {
    let (a, c) = (android.clone(), ctx.clone());
    app.hooks.file_dialog_async = Some(Box::new(move |dialog, done| ask(&a, &c, dialog, done)));

    // The blocking hooks (Settings paths and the like) wait on the same dialogs.
    let blocking = |android: &AndroidApp, ctx: &egui::Context| {
        let (a, c) = (android.clone(), ctx.clone());
        move |dialog: FileDialog| -> Vec<String> {
            let (tx, rx) = mpsc::channel();
            ask(&a, &c, dialog, Box::new(move |paths| drop(tx.send(paths))));
            rx.recv_timeout(BLOCKING_WAIT).unwrap_or_default()
        }
    };
    let b = blocking(android, ctx);
    app.hooks.pick_files = Some(Box::new(move |exts: &[&str]| b(FileDialog::Open { exts: exts.iter().map(|e| e.to_string()).collect(), many: true })));
    let b = blocking(android, ctx);
    app.hooks.pick_save = Some(Box::new(move |name: &str| b(FileDialog::save(name)).into_iter().next()));
    let b = blocking(android, ctx);
    app.hooks.pick_open_project = Some(Box::new(move || b(FileDialog::OpenProject).into_iter().next()));
    let b = blocking(android, ctx);
    app.hooks.pick_folder = Some(Box::new(move || b(FileDialog::Folder).into_iter().next()));
    let b = blocking(android, ctx);
    app.hooks.pick_save_file = Some(Box::new(move |name: &str, _ext: &str| b(FileDialog::save(name)).into_iter().next()));
}

/// Show `dialog`; `done` gets the paths (empty: cancelled) from a Java thread.
fn ask(android: &AndroidApp, ctx: &egui::Context, dialog: FileDialog, done: DialogDone) {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed) % 1000;
    pending(|p| p.insert(id, Pending { done, ctx: ctx.clone() }));
    let (kind, name, many) = match &dialog {
        FileDialog::Open { many, .. } => (KIND_OPEN, String::new(), *many),
        FileDialog::OpenProject => (KIND_OPEN_PROJECT, String::new(), false),
        FileDialog::Folder => (KIND_FOLDER, String::new(), false),
        FileDialog::Save { default, .. } => {
            (KIND_SAVE, std::path::Path::new(default).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), false)
        }
    };
    if start(android, id, kind, &name, many).is_none() {
        log::error!("file dialog: could not start the document picker");
        answer(id, Vec::new());
    }
}

/// Call `MainActivity.startPicker` (an instance method, so the app's class loader is not needed).
fn start(android: &AndroidApp, id: jint, kind: jint, name: &str, many: bool) -> Option<()> {
    // SAFETY: android-activity hands out the process's JavaVM pointer, valid for its lifetime.
    let vm = unsafe { JavaVM::from_raw(android.vm_as_ptr().cast()) }.ok()?;
    let mut env = vm.attach_current_thread().ok()?;
    // SAFETY: the pointer is the activity's global reference; JObject::from_raw does not free it.
    let activity = unsafe { JObject::from_raw(android.activity_as_ptr().cast()) };
    let result = call_start(&mut env, &activity, id, kind, name, many);
    if result.is_err() {
        // A Java exception left pending would abort the process at the next JNI call.
        let _ = env.exception_clear();
    }
    result.ok()
}

fn call_start(env: &mut JNIEnv, activity: &JObject, id: jint, kind: jint, name: &str, many: bool) -> jni::errors::Result<()> {
    let jname = env.new_string(name)?;
    let mimes = env.new_object_array(0, "java/lang/String", JObject::null())?;
    env.call_method(
        activity,
        "startPicker",
        "(IILjava/lang/String;[Ljava/lang/String;Z)V",
        &[JValue::Int(id), JValue::Int(kind), JValue::Object(&jname), JValue::Object(&mimes), JValue::Bool(u8::from(many))],
    )?;
    Ok(())
}

/// Hand `paths` to the dialog `id` and wake the interface.
fn answer(id: jint, paths: Vec<String>) {
    if let Some(Pending { done, ctx }) = pending(|p| p.remove(&id)) {
        done(paths);
        ctx.request_repaint();
    }
}

/// `MainActivity.onPicked(int id, String[] paths)`: the person chose (or cancelled: no paths).
#[unsafe(no_mangle)]
pub extern "system" fn Java_ai_storyteller_effectcraft_MainActivity_onPicked<'l>(mut env: JNIEnv<'l>, _this: JObject<'l>, id: jint, paths: JObjectArray<'l>) {
    let paths = read_strings(&mut env, &paths).unwrap_or_default();
    answer(id, paths);
}

fn read_strings(env: &mut JNIEnv, array: &JObjectArray) -> jni::errors::Result<Vec<String>> {
    let len = env.get_array_length(array)?;
    let mut out = Vec::with_capacity(len.max(0) as usize);
    for i in 0..len {
        let item = env.get_object_array_element(array, i)?;
        let s: String = env.get_string(&JString::from(item))?.into();
        out.push(s);
    }
    Ok(out)
}
