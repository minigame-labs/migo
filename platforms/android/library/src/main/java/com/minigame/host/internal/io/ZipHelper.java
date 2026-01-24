package com.minigame.host.internal.io;

import android.os.Handler;
import android.os.Looper;
import android.util.Log;

import com.minigame.host.internal.jni.HostNative;

import java.io.*;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.zip.ZipEntry;
import java.util.zip.ZipInputStream;

public final class ZipHelper {

    // Single-thread executor ensures sequential file writes
    private static final ExecutorService EXECUTOR = Executors.newSingleThreadExecutor();
    private static final int BUFFER_SIZE = 64 * 1024;

    private static final Handler MAIN = new Handler(Looper.getMainLooper());
    private static final String TAG = "ZipHelper";

    private ZipHelper() {
    }

    public static void unzip(int hostId, int requestId, String zipPath, String destDir) {
        Log.d(TAG, "unzip: submit hostId=" + hostId + " requestId=" + requestId + " zipPath=" + zipPath + " destDir=" + destDir);
        EXECUTOR.execute(() -> runUnzip(hostId, requestId, zipPath, destDir));
    }

    private static void runUnzip(int hostId, int requestId, String zipPath, String outDir) {
        Log.d(TAG, "runUnzip: start hostId=" + hostId + " requestId=" + requestId);
        File zipFile = new File(zipPath);
        if (!zipFile.isFile()) {
            Log.d(TAG, "runUnzip: zip not found: " + zipPath);
            postError(hostId, requestId, "Zip not found: " + zipPath);
            return;
        }

        File out = new File(outDir);
        if (!out.exists()) {
            boolean ok = out.mkdirs();
            Log.d(TAG, "runUnzip: created outDir=" + outDir + " ok=" + ok);
        }

        final long totalSize = zipFile.length();
        long extracted = 0;

        byte[] buffer = new byte[BUFFER_SIZE];

        try (ZipInputStream zis = new ZipInputStream(new BufferedInputStream(new FileInputStream(zipFile), BUFFER_SIZE))) {

            ZipEntry entry;
            String basePath = out.getCanonicalPath();

            while ((entry = zis.getNextEntry()) != null) {

                Log.d(TAG, "runUnzip: processing entry=" + entry.getName() + " directory=" + entry.isDirectory());

                // --- Path traversal protection ---
                File outFile = new File(out, entry.getName());
                if (!isValidPath(basePath, outFile)) {
                    Log.d(TAG, "runUnzip: invalid path in zip: " + entry.getName());
                    postError(hostId, requestId, "Invalid path in zip: " + entry.getName());
                    return;
                }

                if (entry.isDirectory()) {
                    boolean ok = outFile.mkdirs();
                    Log.d(TAG, "runUnzip: created directory=" + outFile.getAbsolutePath() + " ok=" + ok);
                    continue;
                }

                // --- Ensure parent exists ---
                File parent = outFile.getParentFile();
                if (parent != null && !parent.exists()) {
                    boolean ok = parent.mkdirs();
                    Log.d(TAG, "runUnzip: created parent dir=" + parent.getAbsolutePath() + " ok=" + ok);
                }

                // --- Write file ---
                try (BufferedOutputStream bos = new BufferedOutputStream(new FileOutputStream(outFile), BUFFER_SIZE)) {

                    int read;
                    while ((read = zis.read(buffer)) != -1) {
                        bos.write(buffer, 0, read);
                        extracted += read;

                        long finalExtracted = extracted;

                        // TODO
                        // MAIN.post(() -> HostBridge.getInstance().onUnzipProgress(hostId, requestId, finalExtracted, totalSize));
                    }
                    Log.d(TAG, "runUnzip: wrote file=" + outFile.getAbsolutePath() + " bytes=" + outFile.length());
                }

                zis.closeEntry();
            }

        } catch (IOException e) {
            Log.d(TAG, "runUnzip: IOException: " + e.toString(), e);
            postError(hostId, requestId, e.toString());
            return;
        }

        Log.d(TAG, "runUnzip: completed hostId=" + hostId + " requestId=" + requestId + " totalExtracted=" + extracted);
        MAIN.post(() -> HostNative.onUnzipDone(hostId, requestId));
    }

    private static boolean isValidPath(String baseCanonical, File target) {
        try {
            String targetPath = target.getCanonicalPath();
            boolean ok = targetPath.startsWith(baseCanonical + File.separator);
            Log.d(TAG, "isValidPath: target=" + targetPath + " base=" + baseCanonical + " ok=" + ok);
            return ok;
        } catch (IOException e) {
            Log.d(TAG, "isValidPath: IOException", e);
            return false;
        }
    }

    private static void postError(int hostId, int requestId, String msg) {
        Log.d(TAG, "postError: hostId=" + hostId + " requestId=" + requestId + " msg=" + msg);
        //  TODO:
        //  MAIN.post(() -> HostBridge.getInstance().onUnzipError(hostId, requestId, msg));
    }
}
