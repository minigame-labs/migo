package com.migo.runtime.internal;

import java.io.File;
import java.io.FileInputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.RandomAccessFile;
import java.nio.charset.Charset;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;

/**
 * Decides whether a host-delivered engine binary may be handed to the linker.
 * <p>
 * Only files a {@link com.migo.runtime.NativeLibraryProvider} supplies are
 * verified. A {@code libmigo.so} packaged inside the APK is covered by the
 * package signature the platform already checked, and re-hashing ~45 MB on
 * every cold start would buy nothing while sitting squarely on the path to
 * first frame.
 *
 * <h3>Why the result is remembered</h3>
 * SHA-256 over ~45 MB costs tens of milliseconds on the devices this SDK
 * targets, and it lands on the cold-start path every single launch. The result
 * is therefore recorded beside the binary and reused while the file's identity
 * (length + last-modified) is unchanged. This does not weaken the check: an
 * attacker able to write the marker already holds code execution as the app's
 * own uid, and could have replaced the binary directly. What the marker
 * protects against is the failure that actually happens -- a truncated
 * download, a half-written replacement, a mirror serving the previous release
 * -- and every one of those changes length or mtime.
 *
 * @hide
 */
public final class NativeArtifactVerifier {

    /** Marker suffix. Kept beside the binary so removing the binary removes its claim. */
    static final String MARKER_SUFFIX = ".migo-verified";

    private static final Charset UTF_8 = Charset.forName("UTF-8");
    private static final int BUFFER_BYTES = 1 << 20;

    private NativeArtifactVerifier() {}

    /** Why a delivered file was rejected, or {@link #ACCEPTED}. */
    public static final class Outcome {
        private final boolean accepted;
        private final String reason;
        private final boolean digestComputed;

        private Outcome(boolean accepted, String reason, boolean digestComputed) {
            this.accepted = accepted;
            this.reason = reason;
            this.digestComputed = digestComputed;
        }

        public boolean accepted() {
            return accepted;
        }

        /** Human-readable rejection reason; {@code null} when accepted. */
        public String reason() {
            return reason;
        }

        /** True when this call actually hashed the file rather than reusing a marker. */
        public boolean digestComputed() {
            return digestComputed;
        }

        @Override
        public String toString() {
            return accepted
                    ? "Outcome{accepted, digestComputed=" + digestComputed + "}"
                    : "Outcome{rejected: " + reason + "}";
        }
    }

    private static final Outcome ACCEPTED_CACHED = new Outcome(true, null, false);
    private static final Outcome ACCEPTED_HASHED = new Outcome(true, null, true);

    private static Outcome reject(String reason) {
        return new Outcome(false, reason, false);
    }

    /**
     * Verify {@code file} against {@code expectation}, reusing a previous
     * result when the file's identity is unchanged.
     */
    public static Outcome verify(File file, NativeArtifactExpectation expectation) {
        if (expectation == null) {
            // Refusing here is the point. A provider-delivered binary with no
            // manifest to check it against is arbitrary native code from an
            // unverified source; loading it "because it is probably fine" is
            // the whole vulnerability.
            return reject("no artifact manifest to verify against; refusing to load an "
                    + "unverified engine binary");
        }
        if (file == null) {
            return reject("provider returned no file for " + expectation.abi());
        }
        if (!file.isFile()) {
            return reject("delivered engine binary is not a file: " + file.getPath());
        }
        if (!file.canRead()) {
            return reject("delivered engine binary is not readable: " + file.getPath());
        }
        long length = file.length();
        if (length == 0L) {
            return reject("delivered engine binary is empty: " + file.getPath());
        }

        long modified = file.lastModified();
        Marker marker = Marker.read(markerFor(file));
        if (marker != null && marker.matches(expectation.sha256(), length, modified)) {
            return ACCEPTED_CACHED;
        }

        String actual;
        try {
            actual = sha256(file);
        } catch (IOException e) {
            return reject("could not read the delivered engine binary: " + e);
        }
        if (!actual.equals(expectation.sha256())) {
            return reject("engine binary does not match this SDK build: expected sha256 "
                    + expectation.sha256() + " for " + expectation.abi() + ", got " + actual
                    + ". The delivered file is a different build, a partial download, or "
                    + "for another ABI");
        }
        Marker.write(markerFor(file), expectation.sha256(), length, modified);
        return ACCEPTED_HASHED;
    }

    static File markerFor(File binary) {
        return new File(binary.getPath() + MARKER_SUFFIX);
    }

    /** Lowercase hex SHA-256 of a file's contents. */
    public static String sha256(File file) throws IOException {
        MessageDigest digest;
        try {
            digest = MessageDigest.getInstance("SHA-256");
        } catch (NoSuchAlgorithmException e) {
            throw new IOException("SHA-256 unavailable", e);
        }
        byte[] buffer = new byte[BUFFER_BYTES];
        InputStream in = new FileInputStream(file);
        try {
            int read;
            while ((read = in.read(buffer)) != -1) {
                digest.update(buffer, 0, read);
            }
        } finally {
            closeQuietly(in);
        }
        return hex(digest.digest());
    }

    static String hex(byte[] bytes) {
        char[] out = new char[bytes.length * 2];
        char[] alphabet = "0123456789abcdef".toCharArray();
        for (int i = 0; i < bytes.length; i++) {
            int value = bytes[i] & 0xFF;
            out[i * 2] = alphabet[value >>> 4];
            out[i * 2 + 1] = alphabet[value & 0x0F];
        }
        return new String(out);
    }

    private static void closeQuietly(java.io.Closeable closeable) {
        try {
            closeable.close();
        } catch (IOException ignored) {
            // Nothing actionable: the digest is already computed or already failed.
        }
    }

    /** The recorded claim: this exact file, at this length and mtime, hashed to this. */
    static final class Marker {
        final String sha256;
        final long length;
        final long modified;

        Marker(String sha256, long length, long modified) {
            this.sha256 = sha256;
            this.length = length;
            this.modified = modified;
        }

        boolean matches(String expectedSha256, long actualLength, long actualModified) {
            return sha256.equals(expectedSha256)
                    && length == actualLength
                    && modified == actualModified;
        }

        static Marker read(File markerFile) {
            if (!markerFile.isFile()) {
                return null;
            }
            byte[] raw = new byte[256];
            int total = 0;
            InputStream in = null;
            try {
                in = new FileInputStream(markerFile);
                int read;
                while (total < raw.length && (read = in.read(raw, total, raw.length - total)) != -1) {
                    total += read;
                }
            } catch (IOException e) {
                return null;
            } finally {
                if (in != null) {
                    closeQuietly(in);
                }
            }
            String[] fields = new String(raw, 0, total, UTF_8).trim().split(" ");
            if (fields.length != 3 || !NativeArtifactExpectation.isSha256(fields[0])) {
                return null;
            }
            try {
                return new Marker(fields[0], Long.parseLong(fields[1]), Long.parseLong(fields[2]));
            } catch (NumberFormatException e) {
                return null;
            }
        }

        static void write(File markerFile, String sha256, long length, long modified) {
            // A marker that cannot be written is not an error: verification
            // already passed, and the only cost is hashing again next launch.
            RandomAccessFile out = null;
            try {
                File temporary = new File(markerFile.getPath() + ".tmp");
                out = new RandomAccessFile(temporary, "rw");
                out.setLength(0L);
                out.write((sha256 + " " + length + " " + modified).getBytes(UTF_8));
                out.getFD().sync();
                out.close();
                out = null;
                if (!temporary.renameTo(markerFile)) {
                    // A partially visible marker is worse than none: it would be
                    // read back as a claim about bytes nobody checked.
                    if (!temporary.delete()) {
                        temporary.deleteOnExit();
                    }
                }
            } catch (IOException ignored) {
                // Fall through: next launch hashes again.
            } finally {
                if (out != null) {
                    closeQuietly(out);
                }
            }
        }
    }
}
