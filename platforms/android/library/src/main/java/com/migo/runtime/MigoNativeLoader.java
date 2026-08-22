package com.migo.runtime;

import android.content.Context;
import android.content.res.AssetManager;
import android.os.Build;
import android.util.Log;

import com.migo.runtime.internal.NativeArtifactExpectation;
import com.migo.runtime.internal.NativeArtifactManifests;
import com.migo.runtime.internal.NativeLinker;
import com.migo.runtime.internal.NativeLoadCoordinator;

import java.io.IOException;
import java.io.InputStream;

/**
 * Controls how the Migo engine binary enters the process.
 *
 * <p>By default nothing here needs calling: {@code libmigo.so} ships inside the
 * AAR, {@code System.loadLibrary} finds it, and this class stays out of the
 * way. It exists for hosts that would rather not carry ~17 MB of download and
 * ~45 MB of install for users who may never open a mini-game.
 *
 * <p>To take that path, depend on the {@code -nojni} AAR — the same build with
 * {@code jni/**} removed — and install a {@link NativeLibraryProvider}:
 *
 * <pre>{@code
 * // Application.onCreate(), before any other Migo call
 * MigoNativeLoader.setProvider(this, abi -> {
 *     File engine = new File(getNoBackupFilesDir(), abi + "/libmigo.so");
 *     return engine.isFile() ? engine : null;
 * });
 *
 * // When the user opens a mini-game and the engine is not on disk yet:
 * MigoNativeLoader.RequiredArtifact want = MigoNativeLoader.requiredArtifact(this);
 * // download want.abi's binary from wherever you host it, verify want.sha256,
 * // move it into place atomically, then start the game as usual.
 * }</pre>
 *
 * <p>The download itself is deliberately yours: on Google Play the only
 * compliant source is Play Feature Delivery, while other stores have no such
 * mechanism and expect you to host the file. A downloader baked in here would
 * be wrong for one of the two.
 */
public final class MigoNativeLoader {

    private static final String TAG = "MigoNativeLoader";
    private static final String LIBRARY_NAME = "migo";

    /** ABIs an engine slice is published for. */
    private static final String ABI_ARM64 = "arm64-v8a";
    private static final String ABI_X86_64 = "x86_64";

    private static final NativeLoadCoordinator COORDINATOR =
            new NativeLoadCoordinator(NativeLinker.SYSTEM, LIBRARY_NAME);

    private static volatile Context sContext;

    private MigoNativeLoader() {}

    /** Where the engine load stands. */
    public enum State {
        /** No load has succeeded yet. */
        NOT_LOADED,
        /** The engine is in the process. */
        LOADED,
        /** The last attempt failed. Another may be made — see {@link #lastError()}. */
        FAILED
    }

    /** What a delivered engine binary must be, for this device and this SDK build. */
    public static final class RequiredArtifact {
        /** The ABI to fetch, {@code arm64-v8a} or {@code x86_64}. */
        public final String abi;
        /** Lowercase hex SHA-256 the delivered {@code libmigo.so} must have. */
        public final String sha256;
        /** The slice's artifact identity, as published in the release manifest. */
        public final String artifactId;
        /** The SDK version this expectation belongs to. */
        public final String sdkVersion;

        RequiredArtifact(String abi, String sha256, String artifactId, String sdkVersion) {
            this.abi = abi;
            this.sha256 = sha256;
            this.artifactId = artifactId;
            this.sdkVersion = sdkVersion;
        }

        @Override
        public String toString() {
            return "RequiredArtifact{abi=" + abi + ", sha256=" + sha256
                    + ", artifactId=" + artifactId + ", sdkVersion=" + sdkVersion + "}";
        }
    }

    /**
     * Install the host's engine delivery hook.
     *
     * <p>Call before any other Migo API. Once the engine is loaded a provider
     * can no longer affect anything, so installing one then throws rather than
     * leaving you believing it took effect.
     *
     * @param context any context; the application context is retained
     * @param provider the hook, or {@code null} to return to loading from the APK
     * @throws IllegalStateException if the engine is already loaded
     */
    public static void setProvider(Context context, NativeLibraryProvider provider) {
        if (context == null) {
            throw new IllegalArgumentException("context is required to read the artifact manifest");
        }
        sContext = context.getApplicationContext();
        COORDINATOR.setProvider(provider);
    }

    /** @return where the engine load stands. */
    public static State state() {
        switch (COORDINATOR.state()) {
            case LOADED:
                return State.LOADED;
            case FAILED:
                return State.FAILED;
            default:
                return State.NOT_LOADED;
        }
    }

    /** @return the last load failure, or {@code null}. Cleared by a successful load. */
    public static Throwable lastError() {
        return COORDINATOR.error();
    }

    /**
     * What this device needs, so the host can fetch and check it before
     * handing it over. Reads the manifest embedded in the AAR; does not load
     * the engine and does not touch the network.
     *
     * @return the expectation, or {@code null} if this build carries no
     *         artifact manifest (an unofficial or locally assembled AAR)
     */
    public static RequiredArtifact requiredArtifact(Context context) {
        String abi = deviceAbi();
        NativeArtifactExpectation expectation =
                readExpectation(context == null ? sContext : context, abi);
        if (expectation == null) {
            return null;
        }
        return new RequiredArtifact(
                expectation.abi(), expectation.sha256(), expectation.artifactId(),
                BuildInfo.VERSION);
    }

    /**
     * The ABI whose engine slice this device needs.
     *
     * <p>The first supported ABI wins, matching how the platform itself picks
     * which {@code jni/<abi>} directory to use. A device reporting only ABIs
     * Migo publishes no slice for gets {@code arm64-v8a}, so the failure is a
     * readable hash mismatch rather than a silent wrong-architecture load.
     */
    public static String deviceAbi() {
        String[] supported = Build.SUPPORTED_ABIS;
        if (supported != null) {
            for (int i = 0; i < supported.length; i++) {
                if (ABI_ARM64.equals(supported[i]) || ABI_X86_64.equals(supported[i])) {
                    return supported[i];
                }
            }
        }
        return ABI_ARM64;
    }

    // ==================== SDK-internal ====================

    /**
     * Load the engine if it is not loaded already.
     *
     * @return true when the engine is in the process
     */
    static boolean ensureLoaded() {
        boolean loaded = COORDINATOR.ensureLoaded(deviceAbi(), EXPECTATIONS);
        if (!loaded) {
            Throwable error = COORDINATOR.error();
            Log.e(TAG, "Failed to load native library: "
                    + (error == null ? "unknown" : error.getMessage()));
        }
        return loaded;
    }

    /** Where the loaded engine came from, for diagnostics. */
    static String loadedFrom() {
        return COORDINATOR.loadedFrom();
    }

    private static final NativeLoadCoordinator.ExpectationSource EXPECTATIONS =
            new NativeLoadCoordinator.ExpectationSource() {
                @Override
                public NativeArtifactExpectation expectationFor(String abi) {
                    return readExpectation(sContext, abi);
                }
            };

    private static NativeArtifactExpectation readExpectation(Context context, String abi) {
        if (context == null) {
            return null;
        }
        AssetManager assets = context.getAssets();
        if (assets == null) {
            return null;
        }
        InputStream in = null;
        try {
            in = assets.open(NativeArtifactManifests.sliceAssetPath(abi));
            return NativeArtifactManifests.parseSlice(in, abi);
        } catch (IOException e) {
            Log.w(TAG, "No usable artifact manifest for " + abi + ": " + e.getMessage());
            return null;
        } finally {
            if (in != null) {
                try {
                    in.close();
                } catch (IOException ignored) {
                    // The manifest is already parsed or already failed.
                }
            }
        }
    }

}
