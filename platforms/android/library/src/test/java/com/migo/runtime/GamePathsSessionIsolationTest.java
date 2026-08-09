package com.migo.runtime;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNotEquals;
import static org.junit.Assert.assertTrue;

import java.io.File;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.util.HashSet;
import java.util.Set;
import java.util.function.IntPredicate;

import org.junit.Rule;
import org.junit.Test;
import org.junit.rules.TemporaryFolder;

/**
 * Two live sessions of one game, and what keeps their temporary files apart.
 *
 * <p>{@code /tmp} is documented as lasting for a session, but it used to be
 * derived from the game id alone, so two concurrently live sessions of the same
 * game shared one directory and the first to close deleted the other's live files
 * mid-write. That is data loss, and it is reachable by an ordinary host: the
 * engine is built to run several games at once and nothing stops two of them
 * being the same title.
 *
 * <p>This is the one concurrent-session property that can be proved on a host
 * JVM. {@code RuntimeConfig.Builder(String, String, float)} needs no
 * {@code Context}, so real directories on a real filesystem can be built here and
 * the deletion actually performed, rather than the path shape being compared and
 * the deletion assumed.
 */
public final class GamePathsSessionIsolationTest {

    @Rule
    public final TemporaryFolder folder = new TemporaryFolder();

    private RuntimeConfig config() throws IOException {
        return new RuntimeConfig.Builder(
                        folder.newFolder("cache").getAbsolutePath(),
                        folder.newFolder("files").getAbsolutePath(),
                        1.0f)
                .build();
    }

    private static void write(File directory, String name) throws IOException {
        assertTrue("could not create " + directory, directory.mkdirs() || directory.isDirectory());
        Files.write(new File(directory, name).toPath(), "payload".getBytes(StandardCharsets.UTF_8));
    }

    private static IntPredicate live(int... sessionIds) {
        Set<Integer> ids = new HashSet<>();
        for (int id : sessionIds) {
            ids.add(id);
        }
        return ids::contains;
    }

    @Test
    public void twoSessionsOfOneGameGetSeparateTempDirectories() throws IOException {
        RuntimeConfig config = config();
        GamePaths first = new GamePaths(config, "same-game", 1);
        GamePaths second = new GamePaths(config, "same-game", 2);

        assertNotEquals(first.getTempDir(), second.getTempDir());
        // Distinct is not enough: `tmp/1` and `tmp/1/2` are distinct and one
        // recursive delete still takes both.
        assertFalse(first.getTempDir().toPath().startsWith(second.getTempDir().toPath()));
        assertFalse(second.getTempDir().toPath().startsWith(first.getTempDir().toPath()));

        // The rest is per game deliberately: two sessions of one title are one
        // save file, and splitting these would give the same game two.
        assertEquals(first.getUserDataDir(), second.getUserDataDir());
        assertEquals(first.getCodeDir(), second.getCodeDir());
        assertEquals(first.getCacheDir(), second.getCacheDir());
    }

    /** The defect itself, performed rather than described. */
    @Test
    public void closingOneSessionLeavesTheOtherSessionsTempFiles() throws IOException {
        RuntimeConfig config = config();
        GamePaths closing = new GamePaths(config, "same-game", 1);
        GamePaths surviving = new GamePaths(config, "same-game", 2);
        closing.ensureDirectories();
        surviving.ensureDirectories();

        File survivorFile = new File(surviving.getTempDir(), "in-flight.dat");
        Files.write(survivorFile.toPath(), "payload".getBytes(StandardCharsets.UTF_8));
        write(new File(closing.getTempDir(), "nested"), "doomed.dat");

        closing.cleanupTemp();

        assertTrue("the closing session deleted a live session's file", survivorFile.isFile());
        assertFalse(new File(closing.getTempDir(), "nested").exists());
        assertTrue("the closing session's own directory should remain, empty",
                closing.getTempDir().isDirectory());
    }

    @Test
    public void theSweepRemovesTemporaryDirectoriesNoLiveSessionOwns() throws IOException {
        RuntimeConfig config = config();
        GamePaths starting = new GamePaths(config, "same-game", 3);
        File tempRoot = starting.getTempDir().getParentFile();

        write(new File(tempRoot, "2"), "live.dat");      // another live session
        write(new File(tempRoot, "9"), "abandoned.dat"); // died without teardown
        write(new File(tempRoot, "3"), "stale.dat");     // this id, previous process

        starting.sweepAbandonedTemp(live(2));

        assertTrue("the sweep removed a live session's directory",
                new File(new File(tempRoot, "2"), "live.dat").isFile());
        assertFalse(new File(tempRoot, "9").exists());
        assertFalse("a directory left at this session's own id must not be inherited",
                new File(tempRoot, "3").exists());
    }

    /**
     * Files written directly into the temp root are what a build from before the
     * split left behind, and no session id owns them.
     */
    @Test
    public void theSweepRemovesTemporaryFilesFromBeforeTheSplit() throws IOException {
        RuntimeConfig config = config();
        GamePaths starting = new GamePaths(config, "same-game", 1);
        File tempRoot = starting.getTempDir().getParentFile();
        assertTrue(tempRoot.mkdirs());
        File legacy = new File(tempRoot, "downloaded.bin");
        Files.write(legacy.toPath(), "payload".getBytes(StandardCharsets.UTF_8));

        starting.sweepAbandonedTemp(live(1));

        assertFalse(legacy.exists());
    }

    /** A session whose temp root does not exist yet has nothing to sweep. */
    @Test
    public void theSweepToleratesAnAbsentTempRoot() throws IOException {
        GamePaths starting = new GamePaths(config(), "same-game", 1);
        assertFalse(starting.getTempDir().getParentFile().exists());

        starting.sweepAbandonedTemp(live(1));
    }

    /**
     * The sweep walks the temp root, so the temp root must not be the cache root:
     * the subpackage install store and the install staging directories live
     * directly under that, and {@code /cache} is a subdirectory of it precisely so
     * a game cannot decide which package bytes a later session mounts.
     */
    @Test
    public void theSweptDirectoryCannotReachTheInstallStore() throws IOException {
        RuntimeConfig config = config();
        GamePaths starting = new GamePaths(config, "same-game", 1);
        File tempRoot = starting.getTempDir().getParentFile();

        assertNotEquals(starting.getCacheDir(), tempRoot);
        assertEquals(starting.getCacheDir(), tempRoot.getParentFile());

        write(starting.getCacheDir(), "install-record.json");
        starting.sweepAbandonedTemp(live());

        assertTrue("the sweep reached the per-game cache root",
                new File(starting.getCacheDir(), "install-record.json").isFile());
    }
}
