package com.tonestrand.app;

import android.Manifest;
import android.content.ClipData;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.database.Cursor;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Environment;
import android.provider.DocumentsContract;
import android.provider.OpenableColumns;

import com.google.androidgamesdk.GameActivity;

import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;

/**
 * GameActivity plus what the Rust side cannot do itself: the system file picker (Open and Save as,
 * through the Storage Access Framework), the storage permission on Android 10 and older, and the
 * path of the shared Documents folder. Picked documents are copied into the app's cache folder
 * (the app only sees plain paths) and saved files are written there first, then copied to the
 * chosen document. The methods called from Rust may run on any thread; anything that starts an
 * activity or asks for a permission is posted to the UI thread.
 */
public class MainActivity extends GameActivity {
    private static final int PERMISSION_CODE = 7001;
    private static final int FIRST_CODE = 100;
    private static final int LAST_CODE = 60000;

    private static final class Req {
        final long id;
        final boolean save;

        Req(long id, boolean save) {
            this.id = id;
            this.save = save;
        }
    }

    private final Object lock = new Object();
    private final Map<Integer, Req> requests = new HashMap<>();
    private final Map<Long, Uri> saveTargets = new HashMap<>();
    private int nextCode = FIRST_CODE;

    static native void nativePicked(long id, String[] paths);

    static native void nativePickFailed(long id, String error);

    static native void nativeSaveTarget(long id);

    static native void nativeSaved(long id, String error);

    static native void nativeStorageChanged(boolean granted);

    static native void nativeCreated();

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        // Leftovers of earlier runs; recent ones may still belong to a request in flight.
        File[] old = getCacheDir().listFiles();
        if (old != null) {
            long cutoff = System.currentTimeMillis() - 60L * 60L * 1000L;
            for (File f : old) {
                if ((f.getName().startsWith("pick-") || f.getName().startsWith("save-")) && f.lastModified() < cutoff) {
                    deleteRecursively(f, true);
                }
            }
        }
        super.onCreate(savedInstanceState);
        // The library is loaded by now. A picker answer lost with the old activity must not block
        // the next one.
        nativeCreated();
        if (Build.VERSION.SDK_INT <= 29 && !hasWritePermission()) {
            requestPermissions(new String[] {Manifest.permission.WRITE_EXTERNAL_STORAGE}, PERMISSION_CODE);
        }
    }

    private boolean hasWritePermission() {
        return Build.VERSION.SDK_INT > 29
            || checkSelfPermission(Manifest.permission.WRITE_EXTERNAL_STORAGE) == PackageManager.PERMISSION_GRANTED;
    }

    // ---- Called from Rust ----

    public String sharedDocumentsDir() {
        return Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOCUMENTS).getAbsolutePath();
    }

    public String cacheDir() {
        return getCacheDir().getAbsolutePath();
    }

    public void requestStorage() {
        runOnUiThread(() -> {
            if (Build.VERSION.SDK_INT <= 29 && !hasWritePermission()) {
                requestPermissions(new String[] {Manifest.permission.WRITE_EXTERNAL_STORAGE}, PERMISSION_CODE);
            } else {
                // Nothing to ask (already allowed, or Android 11+ has no such permission): let the
                // Rust side test the folder again.
                nativeStorageChanged(true);
            }
        });
    }

    public void pickOpen(final long id, final boolean multiple, final String[] mimeTypes) {
        runOnUiThread(() -> {
            try {
                Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
                intent.addCategory(Intent.CATEGORY_OPENABLE);
                intent.setType("*/*");
                intent.putExtra(Intent.EXTRA_ALLOW_MULTIPLE, multiple);
                startForResult(intent, new Req(id, false));
            } catch (Throwable e) {
                nativePickFailed(id, String.valueOf(e.getMessage()));
            }
        });
    }

    public void pickSave(final long id, final String suggestedName) {
        runOnUiThread(() -> {
            try {
                Intent intent = new Intent(Intent.ACTION_CREATE_DOCUMENT);
                intent.addCategory(Intent.CATEGORY_OPENABLE);
                intent.setType("*/*");
                intent.putExtra(Intent.EXTRA_TITLE, suggestedName);
                startForResult(intent, new Req(id, true));
            } catch (Throwable e) {
                nativePickFailed(id, String.valueOf(e.getMessage()));
            }
        });
    }

    public void commitSave(final long id, final String tempPath) {
        final Uri target;
        synchronized (lock) {
            target = saveTargets.remove(id);
        }
        if (target == null) {
            nativeSaved(id, "No file chosen");
            return;
        }
        new Thread(() -> {
            String error = null;
            File temp = new File(tempPath);
            try (InputStream in = new FileInputStream(temp);
                 OutputStream out = getContentResolver().openOutputStream(target, "wt")) {
                if (out == null) {
                    throw new IOException("Cannot open the file");
                }
                copy(in, out);
            } catch (Throwable e) {
                error = String.valueOf(e.getMessage());
            }
            deleteRecursively(temp.getParentFile(), true);
            if (error != null) {
                // CREATE_DOCUMENT left an empty document behind.
                try {
                    DocumentsContract.deleteDocument(getContentResolver(), target);
                } catch (Throwable ignored) {
                    // Best effort.
                }
            }
            nativeSaved(id, error);
        }, "tonestrand-save").start();
    }

    // ---- Picker plumbing ----

    private void startForResult(Intent intent, Req req) {
        int code;
        synchronized (lock) {
            code = nextCode++;
            if (nextCode > LAST_CODE) {
                nextCode = FIRST_CODE;
            }
            requests.put(code, req);
        }
        startActivityForResult(intent, code);
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        final Req req;
        synchronized (lock) {
            req = requests.remove(requestCode);
        }
        if (req == null) {
            return;
        }
        if (resultCode != RESULT_OK || data == null) {
            nativePicked(req.id, new String[0]);
            return;
        }
        if (req.save) {
            Uri uri = data.getData();
            if (uri == null) {
                nativePicked(req.id, new String[0]);
                return;
            }
            synchronized (lock) {
                saveTargets.put(req.id, uri);
            }
            nativeSaveTarget(req.id);
            return;
        }
        final List<Uri> uris = new ArrayList<>();
        ClipData clip = data.getClipData();
        if (clip != null) {
            for (int i = 0; i < clip.getItemCount(); i++) {
                uris.add(clip.getItemAt(i).getUri());
            }
        } else if (data.getData() != null) {
            uris.add(data.getData());
        }
        if (uris.isEmpty()) {
            nativePicked(req.id, new String[0]);
            return;
        }
        new Thread(() -> copyPicked(req.id, uris), "tonestrand-pick").start();
    }

    private void copyPicked(long id, List<Uri> uris) {
        File dir = new File(getCacheDir(), "pick-" + id);
        try {
            deleteRecursively(dir, true);
            if (!dir.mkdirs() && !dir.isDirectory()) {
                throw new IOException("Cannot create " + dir);
            }
            Set<String> used = new HashSet<>();
            List<String> paths = new ArrayList<>();
            for (Uri uri : uris) {
                String name = uniqueName(displayName(uri), used);
                File out = new File(dir, name);
                try (InputStream in = getContentResolver().openInputStream(uri);
                     OutputStream os = new FileOutputStream(out)) {
                    if (in == null) {
                        throw new IOException("Cannot open " + name);
                    }
                    copy(in, os);
                }
                paths.add(out.getAbsolutePath());
            }
            nativePicked(id, paths.toArray(new String[0]));
        } catch (Throwable e) {
            deleteRecursively(dir, true);
            nativePickFailed(id, String.valueOf(e.getMessage()));
        }
    }

    private String displayName(Uri uri) {
        String name = null;
        try (Cursor c = getContentResolver().query(uri, new String[] {OpenableColumns.DISPLAY_NAME}, null, null, null)) {
            if (c != null && c.moveToFirst()) {
                name = c.getString(0);
            }
        } catch (Throwable ignored) {
            // Fall back to the last path segment.
        }
        if (name == null || name.isEmpty()) {
            name = uri.getLastPathSegment();
        }
        if (name == null) {
            name = "file";
        }
        name = name.replace('/', '_').replace('\\', '_').replace('\0', '_');
        if (name.isEmpty() || name.equals(".") || name.equals("..")) {
            name = "file";
        }
        return shorten(name);
    }

    /** Keeps a file name under about 200 bytes by cutting the stem, not the extension. */
    private static String shorten(String name) {
        if (name.getBytes(StandardCharsets.UTF_8).length <= 200) {
            return name;
        }
        int dot = name.lastIndexOf('.');
        String ext = dot > 0 && name.length() - dot <= 20 ? name.substring(dot) : "";
        String stem = ext.isEmpty() ? name : name.substring(0, dot);
        while (!stem.isEmpty() && (stem + ext).getBytes(StandardCharsets.UTF_8).length > 200) {
            stem = stem.substring(0, stem.length() - 1);
        }
        return stem.isEmpty() ? "file" + ext : stem + ext;
    }

    /** Two documents with one display name in one pick: "a.hlx", "a (2).hlx", "a (3).hlx". */
    private static String uniqueName(String name, Set<String> used) {
        String candidate = name;
        int dot = name.lastIndexOf('.');
        String stem = dot > 0 ? name.substring(0, dot) : name;
        String ext = dot > 0 ? name.substring(dot) : "";
        int n = 2;
        while (!used.add(candidate)) {
            candidate = stem + " (" + n + ")" + ext;
            n++;
        }
        return candidate;
    }

    private static void copy(InputStream in, OutputStream out) throws IOException {
        byte[] buf = new byte[64 * 1024];
        int n;
        while ((n = in.read(buf)) > 0) {
            out.write(buf, 0, n);
        }
        out.flush();
    }

    private static void deleteRecursively(File f, boolean includeSelf) {
        if (f == null) {
            return;
        }
        File[] children = f.listFiles();
        if (children != null) {
            for (File c : children) {
                deleteRecursively(c, true);
            }
        }
        if (includeSelf) {
            //noinspection ResultOfMethodCallIgnored
            f.delete();
        }
    }

    @Override
    public void onRequestPermissionsResult(int requestCode, String[] permissions, int[] grantResults) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults);
        if (requestCode == PERMISSION_CODE) {
            boolean granted = grantResults.length > 0 && grantResults[0] == PackageManager.PERMISSION_GRANTED;
            nativeStorageChanged(granted);
        }
    }
}
