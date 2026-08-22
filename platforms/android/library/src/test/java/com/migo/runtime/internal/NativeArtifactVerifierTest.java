package com.migo.runtime.internal;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import org.junit.Rule;
import org.junit.Test;
import org.junit.rules.TemporaryFolder;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;

/**
 * Host-JVM tests for the check that stands between a host-delivered file and
 * {@code dlopen}.
 *
 * <p>The interesting cases are the ones that actually happen in the field: a
 * truncated download, a mirror still serving the previous release, a file that
 * was replaced after it was verified once.
 */
public final class NativeArtifactVerifierTest {

    @Rule
    public final TemporaryFolder folder = new TemporaryFolder();

    private File write(String name, String contents) throws IOException {
        File file = new File(folder.getRoot(), name);
        FileOutputStream out = new FileOutputStream(file);
        try {
            out.write(contents.getBytes("UTF-8"));
        } finally {
            out.close();
        }
        return file;
    }

    private NativeArtifactExpectation expect(File file) throws IOException {
        return new NativeArtifactExpectation(
                "arm64-v8a", NativeArtifactVerifier.sha256(file), "artifact-id");
    }

    @Test
    public void aMatchingBinaryIsAcceptedAndTheDigestIsComputedOnce() throws IOException {
        File engine = write("libmigo.so", "engine bytes");
        NativeArtifactExpectation expectation = expect(engine);

        NativeArtifactVerifier.Outcome first = NativeArtifactVerifier.verify(engine, expectation);
        assertTrue(first.accepted());
        assertTrue("the first verification must actually hash the file", first.digestComputed());

        NativeArtifactVerifier.Outcome second = NativeArtifactVerifier.verify(engine, expectation);
        assertTrue(second.accepted());
        assertFalse("a repeat verification of the same file must reuse the marker",
                second.digestComputed());
    }

    /**
     * The marker is an optimisation, never an override. A file replaced after
     * being verified has a new length or mtime, and must be hashed again.
     */
    @Test
    public void aReplacedBinaryIsHashedAgainAndRejected() throws IOException {
        File engine = write("libmigo.so", "engine bytes");
        NativeArtifactExpectation expectation = expect(engine);
        assertTrue(NativeArtifactVerifier.verify(engine, expectation).accepted());

        File replaced = write("libmigo.so", "some other bytes entirely");
        assertTrue("test setup: the replacement must differ in length",
                replaced.length() != 12L);

        NativeArtifactVerifier.Outcome outcome =
                NativeArtifactVerifier.verify(replaced, expectation);
        assertFalse(outcome.accepted());
        assertTrue(outcome.reason().contains("sha256"));
    }

    /**
     * The stale case a length check alone would miss: the previous release and
     * this one can be the same size.
     */
    @Test
    public void aSameLengthReplacementIsStillRejected() throws IOException {
        File engine = write("libmigo.so", "engine bytes");
        NativeArtifactExpectation expectation = expect(engine);
        assertTrue(NativeArtifactVerifier.verify(engine, expectation).accepted());

        File replaced = write("libmigo.so", "engine bytez");
        assertEquals(engine.length(), replaced.length());
        replaced.setLastModified(replaced.lastModified() + 4000L);

        assertFalse(NativeArtifactVerifier.verify(replaced, expectation).accepted());
    }

    @Test
    public void aTruncatedDownloadIsRejected() throws IOException {
        File complete = write("complete.so", "the whole engine");
        NativeArtifactExpectation expectation = new NativeArtifactExpectation(
                "arm64-v8a", NativeArtifactVerifier.sha256(complete), "artifact-id");
        File partial = write("libmigo.so", "the whole eng");

        NativeArtifactVerifier.Outcome outcome =
                NativeArtifactVerifier.verify(partial, expectation);

        assertFalse(outcome.accepted());
        assertTrue(outcome.reason().contains("partial download"));
    }

    @Test
    public void anEmptyFileIsRejectedWithoutHashing() throws IOException {
        File engine = write("libmigo.so", "");

        NativeArtifactVerifier.Outcome outcome = NativeArtifactVerifier.verify(
                engine,
                new NativeArtifactExpectation(
                        "arm64-v8a",
                        "0000000000000000000000000000000000000000000000000000000000000000",
                        "artifact-id"));

        assertFalse(outcome.accepted());
        assertTrue(outcome.reason().contains("empty"));
    }

    @Test
    public void aMissingFileIsRejected() {
        NativeArtifactVerifier.Outcome outcome = NativeArtifactVerifier.verify(
                new File(folder.getRoot(), "absent.so"),
                new NativeArtifactExpectation(
                        "arm64-v8a",
                        "0000000000000000000000000000000000000000000000000000000000000000",
                        "artifact-id"));

        assertFalse(outcome.accepted());
        assertTrue(outcome.reason().contains("not a file"));
    }

    @Test
    public void aNullFileIsRejected() {
        NativeArtifactVerifier.Outcome outcome = NativeArtifactVerifier.verify(
                null,
                new NativeArtifactExpectation(
                        "arm64-v8a",
                        "0000000000000000000000000000000000000000000000000000000000000000",
                        "artifact-id"));

        assertFalse(outcome.accepted());
    }

    @Test
    public void withoutAnExpectationNothingIsAccepted() throws IOException {
        File engine = write("libmigo.so", "engine bytes");

        NativeArtifactVerifier.Outcome outcome = NativeArtifactVerifier.verify(engine, null);

        assertFalse(outcome.accepted());
        assertTrue(outcome.reason().contains("unverified"));
    }

    /** A corrupt marker must fall back to hashing, not be believed and not throw. */
    @Test
    public void aCorruptMarkerIsIgnored() throws IOException {
        File engine = write("libmigo.so", "engine bytes");
        NativeArtifactExpectation expectation = expect(engine);
        assertTrue(NativeArtifactVerifier.verify(engine, expectation).accepted());
        write("libmigo.so" + NativeArtifactVerifier.MARKER_SUFFIX, "not a marker at all");

        NativeArtifactVerifier.Outcome outcome =
                NativeArtifactVerifier.verify(engine, expectation);

        assertTrue(outcome.accepted());
        assertTrue("a corrupt marker must force a real digest", outcome.digestComputed());
    }

    /**
     * A marker claiming a different digest than the one being asked about is
     * not a hit. Without this the marker would authorise whatever it last saw
     * rather than what this SDK build expects.
     */
    @Test
    public void aMarkerForADifferentExpectationIsNotAHit() throws IOException {
        File engine = write("libmigo.so", "engine bytes");
        assertTrue(NativeArtifactVerifier.verify(engine, expect(engine)).accepted());

        NativeArtifactVerifier.Outcome outcome = NativeArtifactVerifier.verify(
                engine,
                new NativeArtifactExpectation(
                        "arm64-v8a",
                        "1111111111111111111111111111111111111111111111111111111111111111",
                        "artifact-id"));

        assertFalse(outcome.accepted());
    }

    @Test
    public void theDigestMatchesAKnownVector() throws IOException {
        File file = write("abc.bin", "abc");
        assertEquals("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
                NativeArtifactVerifier.sha256(file));
    }
}
