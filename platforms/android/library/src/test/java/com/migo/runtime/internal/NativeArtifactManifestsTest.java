package com.migo.runtime.internal;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;
import static org.junit.Assert.fail;

import org.junit.Test;

import java.io.ByteArrayInputStream;
import java.io.IOException;

/**
 * Host-JVM tests for reading the slice manifest the AAR embeds.
 *
 * <p>The real org.json is on this module's test classpath (android.jar's stub
 * throws "not mocked" for every method), so this exercises the same parser the
 * device runs.
 */
public final class NativeArtifactManifestsTest {

    private static final String SHA =
            "d12ad5a9bf2ff8304e010b641908bdd63ec432d2e382e327a3f1edcad40a48d6";

    private static NativeArtifactExpectation parse(String json) throws IOException {
        return NativeArtifactManifests.parseSlice(
                new ByteArrayInputStream(json.getBytes("UTF-8")), "arm64-v8a");
    }

    private static void rejects(String json, String expectedFragment) {
        try {
            parse(json);
            fail("expected rejection mentioning: " + expectedFragment);
        } catch (IOException expected) {
            assertTrue("message was: " + expected.getMessage(),
                    expected.getMessage().contains(expectedFragment));
        }
    }

    @Test
    public void theAssetPathIsTheOneTheAarPacks() {
        assertEquals("migo/artifacts/slices/arm64-v8a.json",
                NativeArtifactManifests.sliceAssetPath("arm64-v8a"));
        assertEquals("migo/artifacts/slices/x86_64.json",
                NativeArtifactManifests.sliceAssetPath("x86_64"));
    }

    @Test
    public void aRealSliceYieldsTheRuntimeBinaryHash() throws IOException {
        NativeArtifactExpectation expectation = parse("{"
                + "\"schema\":\"migo-artifact-manifest/v1\","
                + "\"artifact_id\":\"" + SHA + "\","
                + "\"hashes\":{\"runtime_binary\":\"" + SHA + "\",\"cxx_runtime\":\"" + SHA + "\"}"
                + "}");

        assertEquals("arm64-v8a", expectation.abi());
        assertEquals(SHA, expectation.sha256());
        assertEquals(SHA, expectation.artifactId());
    }

    @Test
    public void anUnknownSchemaIsRefused() {
        rejects("{\"schema\":\"migo-artifact-manifest/v2\",\"hashes\":{\"runtime_binary\":\""
                + SHA + "\"}}", "unsupported slice manifest schema");
    }

    @Test
    public void aMissingRuntimeBinaryHashIsRefused() {
        rejects("{\"schema\":\"migo-artifact-manifest/v1\",\"hashes\":{\"cxx_runtime\":\""
                + SHA + "\"}}", "runtime_binary");
    }

    @Test
    public void aMalformedHashIsRefused() {
        rejects("{\"schema\":\"migo-artifact-manifest/v1\",\"hashes\":{\"runtime_binary\":\"nope\"}}",
                "runtime_binary");
    }

    /** Uppercase hex would silently never match the lowercase digest we compute. */
    @Test
    public void anUppercaseHashIsRefusedRatherThanNeverMatching() {
        rejects("{\"schema\":\"migo-artifact-manifest/v1\",\"hashes\":{\"runtime_binary\":\""
                + SHA.toUpperCase(java.util.Locale.ROOT) + "\"}}", "runtime_binary");
    }

    @Test
    public void nonJsonIsRefused() {
        rejects("<html>404</html>", "not JSON");
    }

    @Test
    public void aMissingStreamIsRefused() {
        try {
            NativeArtifactManifests.parseSlice(null, "arm64-v8a");
            fail("expected rejection");
        } catch (IOException expected) {
            assertTrue(expected.getMessage().contains("no slice manifest stream"));
        }
    }
}
