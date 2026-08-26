package com.migo.runtime.internal;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertTrue;
import static org.junit.Assert.fail;

import org.junit.Rule;
import org.junit.Test;
import org.junit.rules.TemporaryFolder;

import com.migo.runtime.NativeLibraryProvider;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.util.ArrayList;
import java.util.List;

/**
 * Host-JVM tests for the engine load state machine.
 *
 * <p>Runs without Android and without dlopen: {@link NativeLoadCoordinator}
 * takes its linker as an interface precisely so the delivery rules can be
 * exercised here. Packaging the engine outside the APK turns "the .so is
 * missing / truncated / for the wrong ABI" from a build-time failure into one
 * that only appears on a user's device, so these paths need to be tested
 * somewhere that is not a user's device.
 */
public final class NativeLoadCoordinatorTest {

    @Rule
    public final TemporaryFolder folder = new TemporaryFolder();

    /** Records what it was asked to load and can be told to fail. */
    private static final class FakeLinker implements NativeLinker {
        final List<String> byName = new ArrayList<>();
        final List<String> byPath = new ArrayList<>();
        UnsatisfiedLinkError nameFailure;
        UnsatisfiedLinkError pathFailure;

        @Override
        public void loadLibrary(String name) {
            byName.add(name);
            if (nameFailure != null) {
                throw nameFailure;
            }
        }

        @Override
        public void load(String absolutePath) {
            byPath.add(absolutePath);
            if (pathFailure != null) {
                throw pathFailure;
            }
        }
    }

    private static NativeLoadCoordinator.ExpectationSource expecting(
            final NativeArtifactExpectation expectation) {
        return new NativeLoadCoordinator.ExpectationSource() {
            @Override
            public NativeArtifactExpectation expectationFor(String abi) {
                return expectation;
            }
        };
    }

    private File writeEngine(String contents) throws IOException {
        File file = folder.newFile("libmigo.so");
        FileOutputStream out = new FileOutputStream(file);
        try {
            out.write(contents.getBytes("UTF-8"));
        } finally {
            out.close();
        }
        return file;
    }

    private NativeArtifactExpectation expectationFor(File engine) throws IOException {
        return new NativeArtifactExpectation(
                "arm64-v8a", NativeArtifactVerifier.sha256(engine), "artifact-id");
    }

    // ---- packaged (default) delivery ----

    @Test
    public void withNoProviderTheLibraryIsLoadedByNameFromThePackage() {
        FakeLinker linker = new FakeLinker();
        NativeLoadCoordinator coordinator = new NativeLoadCoordinator(linker, "migo");

        assertTrue(coordinator.ensureLoaded("arm64-v8a", expecting(null)));

        assertEquals(1, linker.byName.size());
        assertEquals("migo", linker.byName.get(0));
        assertTrue(linker.byPath.isEmpty());
        assertEquals(NativeLoadCoordinator.State.LOADED, coordinator.state());
        assertNull(coordinator.error());
    }

    /**
     * The manifest is read only on the provided path. Reading it costs an asset
     * open and a JSON parse on the way to first frame, and the packaged binary
     * is already covered by the package signature -- so a source that throws
     * must never be consulted when no provider is installed.
     */
    @Test
    public void thePackagedPathNeverConsultsTheExpectationSource() {
        FakeLinker linker = new FakeLinker();
        NativeLoadCoordinator coordinator = new NativeLoadCoordinator(linker, "migo");

        assertTrue(coordinator.ensureLoaded("arm64-v8a",
                new NativeLoadCoordinator.ExpectationSource() {
                    @Override
                    public NativeArtifactExpectation expectationFor(String abi) {
                        throw new AssertionError("the packaged path must not read the manifest");
                    }
                }));
    }

    @Test
    public void loadingTwiceDoesNotLoadTwice() {
        FakeLinker linker = new FakeLinker();
        NativeLoadCoordinator coordinator = new NativeLoadCoordinator(linker, "migo");

        assertTrue(coordinator.ensureLoaded("arm64-v8a", expecting(null)));
        assertTrue(coordinator.ensureLoaded("arm64-v8a", expecting(null)));

        assertEquals(1, linker.byName.size());
    }

    @Test
    public void aPackagedLoadFailureIsRecordedAndReportedAsFailed() {
        FakeLinker linker = new FakeLinker();
        linker.nameFailure = new UnsatisfiedLinkError("no libmigo.so in the package");
        NativeLoadCoordinator coordinator = new NativeLoadCoordinator(linker, "migo");

        assertFalse(coordinator.ensureLoaded("arm64-v8a", expecting(null)));

        assertEquals(NativeLoadCoordinator.State.FAILED, coordinator.state());
        assertNotNull(coordinator.error());
        assertNull(coordinator.loadedFrom());
    }

    // ---- provided delivery ----

    @Test
    public void aVerifiedProvidedBinaryIsLoadedByAbsolutePath() throws IOException {
        final File engine = writeEngine("engine bytes");
        FakeLinker linker = new FakeLinker();
        NativeLoadCoordinator coordinator = new NativeLoadCoordinator(linker, "migo");
        coordinator.setProvider(new NativeLibraryProvider() {
            @Override
            public File resolve(String abi) {
                return engine;
            }
        });

        assertTrue(coordinator.ensureLoaded("arm64-v8a", expecting(expectationFor(engine))));

        assertTrue(linker.byName.isEmpty());
        assertEquals(1, linker.byPath.size());
        assertEquals(engine.getAbsolutePath(), linker.byPath.get(0));
        assertEquals(engine.getAbsolutePath(), coordinator.loadedFrom());
    }

    /**
     * The ABI the coordinator was asked for is the ABI the provider is asked
     * for. A coordinator that passed a constant would still load correctly on
     * the majority architecture and hand x86_64 devices an arm64 binary.
     */
    @Test
    public void theProviderIsAskedForTheAbiTheDeviceNeeds() throws IOException {
        final File engine = writeEngine("engine bytes");
        final List<String> asked = new ArrayList<>();
        NativeLoadCoordinator coordinator = new NativeLoadCoordinator(new FakeLinker(), "migo");
        coordinator.setProvider(new NativeLibraryProvider() {
            @Override
            public File resolve(String abi) {
                asked.add(abi);
                return engine;
            }
        });

        coordinator.ensureLoaded("x86_64", expecting(new NativeArtifactExpectation(
                "x86_64", NativeArtifactVerifier.sha256(engine), "artifact-id")));

        assertEquals(1, asked.size());
        assertEquals("x86_64", asked.get(0));
    }

    /** "Not downloaded yet" must be recoverable without restarting the process. */
    @Test
    public void aProviderThatHasNothingYetFailsAndTheNextAttemptSucceeds() throws IOException {
        final File engine = writeEngine("engine bytes");
        final boolean[] delivered = {false};
        FakeLinker linker = new FakeLinker();
        NativeLoadCoordinator coordinator = new NativeLoadCoordinator(linker, "migo");
        coordinator.setProvider(new NativeLibraryProvider() {
            @Override
            public File resolve(String abi) {
                return delivered[0] ? engine : null;
            }
        });
        NativeLoadCoordinator.ExpectationSource source = expecting(expectationFor(engine));

        assertFalse(coordinator.ensureLoaded("arm64-v8a", source));
        assertEquals(NativeLoadCoordinator.State.FAILED, coordinator.state());
        assertTrue(linker.byPath.isEmpty());

        delivered[0] = true;
        assertTrue(coordinator.ensureLoaded("arm64-v8a", source));
        assertEquals(NativeLoadCoordinator.State.LOADED, coordinator.state());
        assertNull(coordinator.error());
    }

    @Test
    public void aBinaryThatDoesNotMatchTheManifestIsNeverHandedToTheLinker() throws IOException {
        final File engine = writeEngine("a different build");
        FakeLinker linker = new FakeLinker();
        NativeLoadCoordinator coordinator = new NativeLoadCoordinator(linker, "migo");
        coordinator.setProvider(new NativeLibraryProvider() {
            @Override
            public File resolve(String abi) {
                return engine;
            }
        });

        assertFalse(coordinator.ensureLoaded("arm64-v8a", expecting(new NativeArtifactExpectation(
                "arm64-v8a",
                "0000000000000000000000000000000000000000000000000000000000000000",
                "artifact-id"))));

        assertTrue("a rejected binary must not reach dlopen", linker.byPath.isEmpty());
        assertEquals(NativeLoadCoordinator.State.FAILED, coordinator.state());
        assertTrue(coordinator.error().getMessage().contains("sha256"));
    }

    /**
     * With no manifest there is nothing to check the delivered bytes against.
     * Loading anyway would make the provider hook an arbitrary-native-code
     * entry point, which is the one outcome this feature must not create.
     */
    @Test
    public void withoutAManifestAProvidedBinaryIsRefused() throws IOException {
        final File engine = writeEngine("engine bytes");
        FakeLinker linker = new FakeLinker();
        NativeLoadCoordinator coordinator = new NativeLoadCoordinator(linker, "migo");
        coordinator.setProvider(new NativeLibraryProvider() {
            @Override
            public File resolve(String abi) {
                return engine;
            }
        });

        assertFalse(coordinator.ensureLoaded("arm64-v8a", expecting(null)));

        assertTrue(linker.byPath.isEmpty());
        assertTrue(coordinator.error().getMessage().contains("unverified"));
    }

    /** Host code that throws must become a readable load failure, not an escaping exception. */
    @Test
    public void aThrowingProviderBecomesALoadFailure() {
        NativeLoadCoordinator coordinator = new NativeLoadCoordinator(new FakeLinker(), "migo");
        coordinator.setProvider(new NativeLibraryProvider() {
            @Override
            public File resolve(String abi) {
                throw new IllegalStateException("host downloader exploded");
            }
        });

        assertFalse(coordinator.ensureLoaded("arm64-v8a", expecting(null)));

        assertEquals(NativeLoadCoordinator.State.FAILED, coordinator.state());
        assertEquals("host downloader exploded", coordinator.error().getMessage());
    }

    @Test
    public void installingAProviderAfterTheEngineIsLoadedIsRefused() {
        NativeLoadCoordinator coordinator = new NativeLoadCoordinator(new FakeLinker(), "migo");
        assertTrue(coordinator.ensureLoaded("arm64-v8a", expecting(null)));

        try {
            coordinator.setProvider(new NativeLibraryProvider() {
                @Override
                public File resolve(String abi) {
                    return null;
                }
            });
            fail("installing a provider after load must not appear to succeed");
        } catch (IllegalStateException expected) {
            assertTrue(expected.getMessage().contains("already loaded"));
        }
    }

    /** After a failure a provider can still be installed -- that is the fix for the failure. */
    @Test
    public void aProviderMayBeInstalledAfterAFailedLoad() {
        FakeLinker linker = new FakeLinker();
        linker.nameFailure = new UnsatisfiedLinkError("no libmigo.so in the package");
        NativeLoadCoordinator coordinator = new NativeLoadCoordinator(linker, "migo");
        assertFalse(coordinator.ensureLoaded("arm64-v8a", expecting(null)));

        coordinator.setProvider(new NativeLibraryProvider() {
            @Override
            public File resolve(String abi) {
                return null;
            }
        });

        assertNotNull(coordinator.provider());
    }

    /**
     * The symptom of "you took the -nojni AAR and forgot the provider" is
     * dlopen's "library not found", which names neither cause. Both causes it
     * could be must appear in the message a host reads.
     */
    @Test
    public void aMissingPackagedBinaryNamesBothCausesAndTheAbi() {
        FakeLinker linker = new FakeLinker();
        linker.nameFailure = new UnsatisfiedLinkError("dlopen failed: library \"libmigo.so\" not found");
        NativeLoadCoordinator coordinator = new NativeLoadCoordinator(linker, "migo");

        assertFalse(coordinator.ensureLoaded("x86_64", expecting(null)));

        String message = coordinator.error().getMessage();
        assertTrue(message, message.contains("NativeLibraryProvider"));
        assertTrue(message, message.contains("abiFilters"));
        assertTrue(message, message.contains("x86_64"));
    }
}
