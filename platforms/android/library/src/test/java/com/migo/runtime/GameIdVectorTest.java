package com.migo.runtime;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;
import static org.junit.Assert.fail;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;

import org.junit.Test;

/**
 * The Java half of the game identity rule, checked against the table the engine
 * checks itself against.
 *
 * <p>Two implementations enforce one rule -- this class and
 * {@code shared::vfs::game_paths::validate_game_id} -- and the id is a storage
 * isolation boundary, so a disagreement means the SDK accepts a session the
 * engine will refuse, or worse, admits an id that shares a directory with
 * another game on a case-insensitive filesystem. Reading the engine's own vector
 * file is what makes a one-sided change fail: widening either rule without the
 * other leaves this test, or its Rust counterpart, red.
 */
public final class GameIdVectorTest {

    private static final String VECTORS =
            "engine/crates/shared/src/vfs/game-id-vectors.txt";

    /** One vector: the id and whether the rule accepts it. */
    private static final class Vector {
        final String id;
        final boolean accepted;

        Vector(String id, boolean accepted) {
            this.id = id;
            this.accepted = accepted;
        }
    }

    @Test
    public void theJavaGateAgreesWithTheEnginesVectorTable() throws IOException {
        List<Vector> vectors = loadVectors();

        // A table that lost its contents, or every entry of one verdict, would
        // satisfy the loop below while checking nothing.
        assertTrue("vector table shrank: " + vectors.size(), vectors.size() >= 25);
        assertTrue("no accepted vectors", vectors.stream().anyMatch(v -> v.accepted));
        assertTrue("no rejected vectors", vectors.stream().anyMatch(v -> !v.accepted));

        for (Vector vector : vectors) {
            assertEquals("vector " + vector.id,
                    vector.accepted, GamePaths.isValidGameId(vector.id));
        }
    }

    @Test
    public void everyAcceptedIdIsItsOwnLowerCaseSpelling() throws IOException {
        // The property the case rule exists for: two accepted ids that fold
        // together are one directory on Windows and macOS. They cannot fold
        // together if each accepted id is already folded.
        for (Vector vector : loadVectors()) {
            if (!vector.accepted) continue;
            assertEquals("accepted id is not case-canonical",
                    vector.id.toLowerCase(java.util.Locale.ROOT), vector.id);
        }
        assertTrue(GamePaths.isValidGameId("puzzlequest"));
        assertTrue(!GamePaths.isValidGameId("PuzzleQuest"));
    }

    @Test
    public void aNullIdIsRefusedRatherThanCrashing() {
        assertTrue(!GamePaths.isValidGameId(null));
    }

    private static List<Vector> loadVectors() throws IOException {
        Path file = locateVectors();
        List<Vector> vectors = new ArrayList<>();
        for (String raw : Files.readAllLines(file, StandardCharsets.UTF_8)) {
            String line = raw.trim();
            if (line.isEmpty() || line.startsWith("#")) continue;
            String[] fields = line.split("\\s+");
            if (fields.length != 2) {
                fail("malformed vector line: " + line);
            }
            boolean accepted;
            if ("ok".equals(fields[0])) {
                accepted = true;
            } else if ("err".equals(fields[0])) {
                accepted = false;
            } else {
                fail("unknown verdict in: " + line);
                return vectors;
            }
            vectors.add(new Vector(fields[1], accepted));
        }
        return vectors;
    }

    /**
     * Walk up from the test's working directory until the vector file appears.
     *
     * <p>Not a fixed relative path, because Gradle's working directory is the
     * module and a developer running the same test from the repository root
     * would otherwise silently exercise nothing. A missing file fails loudly
     * with the directories that were searched: an unreadable table has to be an
     * error, since a skipped table is a gate that passes for free.
     */
    private static Path locateVectors() {
        StringBuilder searched = new StringBuilder();
        Path directory = Paths.get("").toAbsolutePath();
        while (directory != null) {
            Path candidate = directory.resolve(VECTORS);
            if (Files.isReadable(candidate)) {
                return candidate;
            }
            searched.append("\n  ").append(candidate);
            directory = directory.getParent();
        }
        throw new AssertionError("game id vector table not found, searched:" + searched);
    }
}
