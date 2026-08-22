package com.migo.runtime.internal;

import org.json.JSONException;
import org.json.JSONObject;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.Charset;

/**
 * Reads the per-slice artifact manifest the AAR embeds.
 * <p>
 * The AAR carries {@code assets/migo/artifacts/slices/<abi>.json} whether or
 * not it carries the matching {@code jni/<abi>/libmigo.so}. That is what makes
 * an engine delivered outside the APK checkable offline: the identity of the
 * binary this SDK build was cut against travels with the Java half, so nothing
 * about the check depends on reaching the network or trusting the host's
 * mirror.
 *
 * @hide
 */
public final class NativeArtifactManifests {

    /** Asset path of a slice manifest, relative to the AAR's {@code assets/}. */
    public static final String SLICE_ASSET_PREFIX = "migo/artifacts/slices/";

    private static final Charset UTF_8 = Charset.forName("UTF-8");
    private static final String SLICE_SCHEMA = "migo-artifact-manifest/v1";
    /** A manifest is small; anything larger is not one. */
    private static final int MAX_BYTES = 1 << 20;

    private NativeArtifactManifests() {}

    /** @return the asset path for {@code abi}, e.g. {@code migo/artifacts/slices/arm64-v8a.json}. */
    public static String sliceAssetPath(String abi) {
        return SLICE_ASSET_PREFIX + abi + ".json";
    }

    /**
     * Parse one slice manifest.
     *
     * @throws IOException when the stream is unreadable, is not a slice manifest,
     *                     describes a different ABI, or carries no binary hash
     */
    public static NativeArtifactExpectation parseSlice(InputStream in, String abi)
            throws IOException {
        if (abi == null || abi.isEmpty()) {
            throw new IOException("abi is required to read a slice manifest");
        }
        JSONObject slice;
        try {
            slice = new JSONObject(readUtf8(in));
        } catch (JSONException e) {
            throw new IOException("slice manifest for " + abi + " is not JSON: " + e.getMessage());
        }
        String schema = slice.optString("schema", "");
        if (!SLICE_SCHEMA.equals(schema)) {
            throw new IOException("unsupported slice manifest schema '" + schema + "' for " + abi
                    + "; expected " + SLICE_SCHEMA);
        }
        JSONObject hashes = slice.optJSONObject("hashes");
        String runtimeBinary = hashes == null ? null : hashes.optString("runtime_binary", null);
        if (runtimeBinary == null || !NativeArtifactExpectation.isSha256(runtimeBinary)) {
            throw new IOException("slice manifest for " + abi
                    + " carries no usable hashes.runtime_binary");
        }
        String artifactId = slice.optString("artifact_id", null);
        return new NativeArtifactExpectation(abi, runtimeBinary, artifactId);
    }

    private static String readUtf8(InputStream in) throws IOException {
        if (in == null) {
            throw new IOException("no slice manifest stream");
        }
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        byte[] buffer = new byte[8192];
        int read;
        int total = 0;
        while ((read = in.read(buffer)) != -1) {
            total += read;
            if (total > MAX_BYTES) {
                throw new IOException("slice manifest exceeds " + MAX_BYTES + " bytes");
            }
            out.write(buffer, 0, read);
        }
        return new String(out.toByteArray(), UTF_8);
    }
}
