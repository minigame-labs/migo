package com.migo.runtime.internal;

/**
 * What a host-delivered {@code libmigo.so} must be, for one ABI.
 * <p>
 * Read from the artifact manifest the AAR already embeds at
 * {@code assets/migo/artifacts/slices/<abi>.json}. The manifest travels with
 * the Java SDK rather than with the binary on purpose: an engine delivered
 * outside the APK is verified against what *this* SDK build was cut against,
 * so a stale mirror or a half-finished rollout fails at load instead of
 * presenting as an unexplained crash inside the engine.
 *
 * @hide
 */
public final class NativeArtifactExpectation {

    private final String abi;
    private final String sha256;
    private final String artifactId;

    public NativeArtifactExpectation(String abi, String sha256, String artifactId) {
        if (abi == null || abi.isEmpty()) {
            throw new IllegalArgumentException("abi is required");
        }
        if (sha256 == null || !isSha256(sha256)) {
            throw new IllegalArgumentException("sha256 must be 64 lowercase hex characters");
        }
        this.abi = abi;
        this.sha256 = sha256;
        this.artifactId = artifactId;
    }

    /** The ABI directory name this expectation describes, e.g. {@code arm64-v8a}. */
    public String abi() {
        return abi;
    }

    /** Lowercase hex SHA-256 of the {@code libmigo.so} bytes. */
    public String sha256() {
        return sha256;
    }

    /** The slice's artifact identity, for diagnostics and host-side telemetry. */
    public String artifactId() {
        return artifactId;
    }

    static boolean isSha256(String value) {
        if (value == null || value.length() != 64) {
            return false;
        }
        for (int i = 0; i < 64; i++) {
            char c = value.charAt(i);
            boolean hex = (c >= '0' && c <= '9') || (c >= 'a' && c <= 'f');
            if (!hex) {
                return false;
            }
        }
        return true;
    }

    @Override
    public String toString() {
        return "NativeArtifactExpectation{abi=" + abi + ", sha256=" + sha256
                + ", artifactId=" + artifactId + "}";
    }
}
