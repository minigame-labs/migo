package com.migo.runtime.callback;

/**
 * Host-provided handler for subpackage downloading.
 * <p>
 * Register via {@link com.migo.runtime.GameSession#setSubpackageHandler(SubpackageHandler)}
 * before the game starts. When the game calls {@code loadSubpackage()} or
 * {@code preDownloadSubpackage()} and local files are not available, this handler
 * is invoked to perform the actual download.
 * <p>
 * When no handler is set, download requests fail and {@code loadSubpackage()} falls
 * back to executing local files if present.
 *
 * <p>Contract:
 * <ul>
 *   <li>{@link #download(SubpackageRequest, DownloadCallback)} must eventually invoke
 *       one of the terminal callback methods ({@code onSuccess} or {@code onFailure}).</li>
 *   <li>Progress reports via {@code onProgress} are optional but recommended.</li>
 *   <li>All callback methods may be invoked from any thread.</li>
 * </ul>
 */
public interface SubpackageHandler {

    /**
     * Download a subpackage zip file.
     * <p>
     * The host should download the subpackage as a zip file to a temporary
     * location, then call {@code callback.onSuccess(zipPath)}.  The runtime
     * will ingest the zip into a .mpkg package, validate it, and mount it
     * atomically.  The host does NOT need to extract the zip.
     *
     * @param request  subpackage info (name and root path)
     * @param callback download progress and completion callback
     */
    void download(SubpackageRequest request, DownloadCallback callback);

    /** Describes which subpackage to download. */
    final class SubpackageRequest {
        /** Subpackage name as provided via RuntimeConfig. */
        public final String name;
        /** Normalized root path relative to code dir (e.g. "subpackages/stage1"). */
        public final String root;

        public SubpackageRequest(String name, String root) {
            this.name = name;
            this.root = root;
        }
    }

    interface DownloadCallback {
        /**
         * Report download progress (optional, may be called multiple times).
         *
         * @param progress                  percentage 0-100
         * @param totalBytesWritten         bytes downloaded so far
         * @param totalBytesExpectedToWrite total expected bytes
         */
        void onProgress(int progress, long totalBytesWritten, long totalBytesExpectedToWrite);

        /**
         * Called when download completes successfully.
         * <p>
         * The host downloads the subpackage as a zip file to a temporary
         * location and provides the path here.  The runtime ingests it into
         * a .mpkg package, validates, and mounts it atomically.
         * The host does NOT need to extract the zip.
         *
         * @param zipPath absolute path to the downloaded zip file
         */
        void onSuccess(String zipPath);

        /**
         * Called when download fails.
         *
         * @param reason failure reason
         */
        void onFailure(String reason);
    }
}
