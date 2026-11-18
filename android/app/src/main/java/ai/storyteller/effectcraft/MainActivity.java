package ai.storyteller.effectcraft;

import android.app.NativeActivity;
import android.content.ClipData;
import android.content.ContentResolver;
import android.content.Intent;
import android.database.Cursor;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.provider.OpenableColumns;
import android.util.Log;
import android.view.View;
import android.view.WindowInsets;
import android.view.WindowInsetsController;
import android.webkit.MimeTypeMap;
import android.widget.Toast;

import java.io.File;
import java.io.InputStream;
import java.io.OutputStream;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.Locale;
import java.util.Map;
import java.util.Set;

/**
 * Hosts the Rust app (libeffectcraft_android.so) and gives it what Android keeps in Java: the
 * system document picker. The interface asks for dialogs through Hooks and expects file paths, so
 * this turns picker URIs into paths (see apps/effectcraft-android/src/picker.rs):
 *
 *  - open / import: each document is copied into files/Media and its path returned;
 *  - save: the person picks a document; the path returned is a staging file, copied to that
 *    document whenever it stops changing;
 *  - folder: the app's own external EffectCraft folder.
 */
public class MainActivity extends NativeActivity {
    private static final String TAG = "EffectCraft";
    private static final int REQUEST_BASE = 0x4500;
    private static final int KIND_OPEN = 0, KIND_OPEN_PROJECT = 1, KIND_FOLDER = 2, KIND_SAVE = 3;

    static {
        // NativeActivity loads the library itself; loading it by name as well lets the JVM find
        // the native method below (JNI lookup goes through the class loader's libraries).
        System.loadLibrary("effectcraft_android");
    }

    /** Implemented in Rust: the person chose (no paths: cancelled). */
    private native void onPicked(int id, String[] paths);

    private final Map<Integer, Integer> kinds = new HashMap<>();
    private final Set<String> watched = new HashSet<>();

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        hideSystemBars();
    }

    @Override
    public void onWindowFocusChanged(boolean hasFocus) {
        super.onWindowFocusChanged(hasFocus);
        if (hasFocus) hideSystemBars();
    }

    private void hideSystemBars() {
        if (Build.VERSION.SDK_INT >= 30) {
            getWindow().setDecorFitsSystemWindows(false);
            WindowInsetsController c = getWindow().getInsetsController();
            if (c != null) {
                c.hide(WindowInsets.Type.systemBars());
                c.setSystemBarsBehavior(WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
            }
        } else {
            getWindow().getDecorView().setSystemUiVisibility(
                View.SYSTEM_UI_FLAG_LAYOUT_STABLE | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
                | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN | View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
                | View.SYSTEM_UI_FLAG_FULLSCREEN | View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY);
        }
    }

    /** Called from Rust (picker.rs) on its own thread. */
    public void startPicker(final int id, final int kind, final String name, final String[] mimes, final boolean many) {
        runOnUiThread(new Runnable() {
            @Override
            public void run() {
                if (kind == KIND_FOLDER) {
                    File dir = exportDir();
                    deliver(id, new String[] { dir.getAbsolutePath() });
                    return;
                }
                Intent intent;
                if (kind == KIND_SAVE) {
                    intent = new Intent(Intent.ACTION_CREATE_DOCUMENT);
                    intent.setType(mimeFor(name));
                    intent.putExtra(Intent.EXTRA_TITLE, name);
                } else {
                    intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
                    intent.setType("*/*");
                    intent.putExtra(Intent.EXTRA_ALLOW_MULTIPLE, many && kind == KIND_OPEN);
                }
                intent.addCategory(Intent.CATEGORY_OPENABLE);
                kinds.put(id, kind);
                try {
                    startActivityForResult(intent, REQUEST_BASE + id);
                } catch (Exception e) {
                    Log.e(TAG, "document picker: " + e);
                    kinds.remove(id);
                    deliver(id, new String[0]);
                }
            }
        });
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, final Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        final int id = requestCode - REQUEST_BASE;
        if (id < 0 || id >= 1000) return;
        final Integer kind = kinds.remove(id);
        if (kind == null) return;
        if (resultCode != RESULT_OK || data == null) {
            deliver(id, new String[0]);
            return;
        }
        // Copying a large video takes a while: not on the UI thread.
        new Thread(new Runnable() {
            @Override
            public void run() {
                try {
                    String[] paths = kind == KIND_SAVE ? stageSave(data.getData()) : importAll(data);
                    deliver(id, paths);
                } catch (Exception e) {
                    Log.e(TAG, "document picker: " + e);
                    toast("Could not use that file: " + e.getMessage());
                    deliver(id, new String[0]);
                }
            }
        }, "ec-picker").start();
    }

    private void deliver(int id, String[] paths) {
        onPicked(id, paths);
    }

    // ---- open / import -------------------------------------------------------------------

    private String[] importAll(Intent data) throws Exception {
        ArrayList<Uri> uris = new ArrayList<>();
        ClipData clip = data.getClipData();
        if (clip != null) {
            for (int i = 0; i < clip.getItemCount(); i++) uris.add(clip.getItemAt(i).getUri());
        } else if (data.getData() != null) {
            uris.add(data.getData());
        }
        ArrayList<String> out = new ArrayList<>();
        for (Uri uri : uris) out.add(importOne(uri).getAbsolutePath());
        return out.toArray(new String[0]);
    }

    /** Copy `uri` into files/Media/<id>/<name>; an identical copy already there is reused. */
    private File importOne(Uri uri) throws Exception {
        String name = displayName(uri);
        long size = documentSize(uri);
        File dir = new File(new File(getFilesDir(), "Media"), shortId(uri));
        File target = new File(dir, name);
        if (target.isFile() && size >= 0 && target.length() == size) return target;
        if (!dir.isDirectory() && !dir.mkdirs()) throw new Exception("no space to import " + name);
        toast("Importing " + name + "…");
        File part = new File(dir, name + ".part");
        copy(getContentResolver(), uri, part);
        if (!part.renameTo(target)) throw new Exception("could not import " + name);
        return target;
    }

    // ---- save ----------------------------------------------------------------------------

    /** The staging file for the chosen document, which is kept in sync with it. */
    private String[] stageSave(Uri uri) throws Exception {
        String name = displayName(uri);
        File dir = new File(new File(getFilesDir(), "Exports"), shortId(uri));
        if (!dir.isDirectory() && !dir.mkdirs()) throw new Exception("no space to save " + name);
        File staging = new File(dir, name);
        watch(staging, uri);
        return new String[] { staging.getAbsolutePath() };
    }

    /** Copy `staging` to `uri` each time it has stopped changing (a render is copied when done). */
    private void watch(final File staging, final Uri uri) {
        synchronized (watched) {
            if (!watched.add(staging.getAbsolutePath())) return;
        }
        Thread t = new Thread(new Runnable() {
            @Override
            public void run() {
                long copiedLength = -1, copiedTime = -1, seenLength = -1, seenTime = -1;
                while (true) {
                    try {
                        Thread.sleep(1000);
                    } catch (InterruptedException e) {
                        return;
                    }
                    if (!staging.isFile()) continue;
                    long length = staging.length(), time = staging.lastModified();
                    boolean stable = length == seenLength && time == seenTime;
                    seenLength = length;
                    seenTime = time;
                    if (!stable || (length == copiedLength && time == copiedTime) || length == 0) continue;
                    try {
                        copyOut(staging, uri);
                        copiedLength = length;
                        copiedTime = time;
                        toast("Saved " + staging.getName());
                    } catch (Exception e) {
                        Log.e(TAG, "save to document: " + e);
                        toast("Could not save " + staging.getName() + ": " + e.getMessage());
                    }
                }
            }
        }, "ec-save-sync");
        t.setDaemon(true);
        t.start();
    }

    private void copyOut(File from, Uri to) throws Exception {
        try (InputStream in = new java.io.FileInputStream(from);
             OutputStream out = getContentResolver().openOutputStream(to, "wt")) {
            if (out == null) throw new Exception("the document is gone");
            byte[] buf = new byte[1 << 20];
            for (int n; (n = in.read(buf)) > 0; ) out.write(buf, 0, n);
        }
    }

    // ---- helpers -------------------------------------------------------------------------

    private File exportDir() {
        File base = getExternalFilesDir(null);
        File dir = new File(base != null ? base : getFilesDir(), "EffectCraft");
        if (!dir.isDirectory()) dir.mkdirs();
        return dir;
    }

    private static void copy(ContentResolver r, Uri uri, File to) throws Exception {
        try (InputStream in = r.openInputStream(uri); OutputStream out = new java.io.FileOutputStream(to)) {
            if (in == null) throw new Exception("the document is gone");
            byte[] buf = new byte[1 << 20];
            for (int n; (n = in.read(buf)) > 0; ) out.write(buf, 0, n);
        }
    }

    private String displayName(Uri uri) {
        String name = null;
        try (Cursor c = getContentResolver().query(uri, new String[] { OpenableColumns.DISPLAY_NAME }, null, null, null)) {
            if (c != null && c.moveToFirst()) name = c.getString(0);
        } catch (Exception ignored) {
        }
        if (name == null || name.isEmpty()) name = uri.getLastPathSegment();
        if (name == null || name.isEmpty()) name = "file";
        // The name becomes a path component.
        return name.replace('/', '_').replace('\\', '_');
    }

    private long documentSize(Uri uri) {
        try (Cursor c = getContentResolver().query(uri, new String[] { OpenableColumns.SIZE }, null, null, null)) {
            if (c != null && c.moveToFirst() && !c.isNull(0)) return c.getLong(0);
        } catch (Exception ignored) {
        }
        return -1;
    }

    private static String shortId(Uri uri) {
        return Long.toHexString(uri.toString().hashCode() & 0xffffffffL);
    }

    private static String mimeFor(String name) {
        int dot = name.lastIndexOf('.');
        if (dot > 0) {
            String m = MimeTypeMap.getSingleton().getMimeTypeFromExtension(name.substring(dot + 1).toLowerCase(Locale.ROOT));
            if (m != null) return m;
        }
        // Not application/json etc. for .ecproj: providers would append their extension.
        return "application/octet-stream";
    }

    private void toast(final String text) {
        runOnUiThread(new Runnable() {
            @Override
            public void run() {
                Toast.makeText(MainActivity.this, text, Toast.LENGTH_SHORT).show();
            }
        });
    }
}
