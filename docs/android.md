# EffectCraft on Android

`apps/effectcraft-android` runs the **desktop egui interface, unchanged**, inside a `NativeActivity`
on Android (arm64, Vulkan). Same panels, same timeline, same shortcuts, same engine. Nothing in
`crates/` below L5 learned about Android; the app crate only replaces what the desktop app takes
from the operating system.

> **Status: first port, never compiled by its author.** It was written against the sources
> without a Rust toolchain, Android SDK or network. Expect to fix compile errors on the first build
> (see "Known risks"). Treat it as a starting point, not a release.

## Build

```sh
# once: JDK 17, Android SDK + NDK r27+ (ANDROID_NDK_HOME), Rust 1.95+, Gradle 8.9+
tools/build-android.sh            # debug APK  -> android/app/build/outputs/apk/debug/
tools/build-android.sh release    # release APK
adb install -r android/app/build/outputs/apk/debug/app-debug.apk
adb logcat -s EffectCraft         # the app's log
```

Or run the `android` workflow (GitHub Actions) and download the APK artifact.
The first build changes `Cargo.lock` (adds `jni`, `android_logger`); don't pass `--locked` then.

## How the desktop feel is kept

| Desktop | Android |
|---|---|
| A window 960–1680 points wide | `scale.rs` sets egui's zoom so the screen's long edge is **1280 points** whatever the pixel density; Settings ▸ Appearance ▸ UI Scale (75–200 %) multiplies it. The activity is locked to **landscape** |
| Mouse | Tap = click, long press = right click, drag = drag, two fingers = scroll/zoom (egui's touch handling). A Bluetooth/USB mouse and keyboard work as on a desktop, including all shortcuts |
| OS windows for floating panels | They stay inside the main window (the same mode as the web build): the app calls `set_embed_viewports(true)`, and the menus then leave out "Move Panel to New Window" |
| `rfd` file dialogs | The system document picker (`picker.rs` + `MainActivity.java`), through the same `Hooks` |
| `cpal` audio | `cpal` on Android (oboe), `audio_out.rs` is the desktop file |
| Settings in `~/.config/effectcraft` | `files/EffectCraft` in the app's private storage (`EFFECTCRAFT_CONFIG_DIR`) |
| System fonts in `/usr/share/fonts` | `/system/fonts` is scanned too |
| Crash helper, launch guard, window geometry, control channel, native menu | Left out |
| In-app updater (`craft-update`) | Not built on Android (`ui-egui/build.rs` sets `ec_no_updater`, as for the web build): Help ▸ Check for Updates shows a "not supported" window |

### Files

Android hands out `content://` URIs, the engine needs paths:

* **Import / Open:** the chosen documents are **copied** into the app's `files/Media/…` and that path
  is used. Large footage is therefore stored twice (the copy is reused on the next import).
* **Save / Render output:** you pick a name and place in the system "save" screen; the app writes a staging file and
  copies it to that document each time it stops changing (a render is copied when finished). A toast says
  "Saved <name>".
* **Folder pickers** (Settings paths): the app's own `Android/data/ai.storyteller.effectcraft/files/EffectCraft`.

## Known risks (things to check first)

1. **Dependencies I could not read.** `craft-ui`, `craft-quit` and the forked `egui`/`winit` (git
   dependencies) must build for `aarch64-linux-android`. `craft-update` is already left out on Android.
2. **eframe features.** `android-native-activity` must exist in the forked eframe/winit (upstream has it).
3. **`libc++_shared.so`.** cpal's oboe backend links the shared C++ runtime; recent `cargo-ndk` copies it
   into `jniLibs`. If the app dies at start with "library libc++_shared.so not found", copy it from the NDK.
4. **Memory and speed.** This is a 300-effect compositor with a JavaScript engine; phones with < 8 GB of RAM
   will struggle with big comps. Use Settings ▸ Previews for lower caches.
5. **No soft-keyboard shortcuts.** The UI is unchanged on purpose, so shortcuts that need Ctrl/Alt/Shift
   need a hardware keyboard (menus offer every command).
6. **`xtask ci`** checks asset attribution and house rules; the new icons in `android/…/mipmap-*` are
   resized copies of `assets/app-icon` and need `.attribution` files to pass it.

## The `unsafe` exception

The workspace forbids `unsafe`. The Android app crate does not inherit those lints (see its
`Cargo.toml`) because JNI needs a few small blocks: `JavaVM::from_raw`/`JObject::from_raw` (pointers from
`android-activity`), the exported `Java_…_onPicked` function and `std::env::set_var` at start-up.
Every block has a `SAFETY` comment. All other crates are unchanged.
