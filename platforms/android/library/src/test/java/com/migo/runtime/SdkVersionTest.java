package com.migo.runtime;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import org.junit.Test;

/**
 * {@link MigoRuntime#SDK_VERSION} is a public constant an embedder can read, and
 * it is the baseline {@link MigoRuntime}'s SDK-vs-native skew check compares
 * against. It was once a hardcoded literal that drifted to an early release's
 * number while {@code BuildInfo.VERSION} (from {@code release/VERSION}) moved on;
 * the two frozen strings then made the skew check unfireable.
 *
 * <p>These assertions run on the host JVM. {@code BuildInfo} is generated from
 * {@code release/VERSION} before Java compilation, so if {@code SDK_VERSION} is
 * turned back into a literal this fails here as well as in
 * {@code scripts/test-release-version-contract.sh}.
 */
public class SdkVersionTest {

    @Test
    public void sdkVersionIsTheBuildVersion() {
        assertEquals(
                "SDK_VERSION must be an alias of BuildInfo.VERSION, not its own literal",
                BuildInfo.VERSION,
                MigoRuntime.SDK_VERSION);
    }

    @Test
    public void sdkVersionIsSemverAndNotAPlaceholder() {
        String v = MigoRuntime.SDK_VERSION;
        assertTrue(
                "SDK_VERSION `" + v + "` is not Semantic Versioning shaped",
                v.matches("\\d+\\.\\d+\\.\\d+(?:-[0-9A-Za-z.-]+)?(?:\\+[0-9A-Za-z.-]+)?"));
        assertFalse(
                "SDK_VERSION is the 0.1.x placeholder the drift check exists to catch",
                v.startsWith("0.1.") || v.equals("0.0.0"));
    }

    @Test
    public void getVersionMatchesTheConstant() {
        assertEquals(MigoRuntime.SDK_VERSION, MigoRuntime.getInstance().getVersion());
    }
}
