package com.migo.runtime;

import java.io.File;
import java.util.regex.Pattern;

/**
 * Manages isolated file paths for a specific game.
 * <p>
 * Each game has its own sandboxed directories:
 * <ul>
 *   <li>{@code /user} → User data (saves, preferences) - persistent</li>
 *   <li>{@code /cache} → Cache files - may be cleared by system</li>
 *   <li>{@code /code} → Game code - read only</li>
 *   <li>{@code /tmp} → Temporary files - cleared on session end</li>
 * </ul>
 * <p>
 * {@link #getCacheDir()} is the per-game cache root rather than the directory
 * {@code /cache} resolves to; see its documentation.
 *
 */
public final class GamePaths {

    // Game ID validation: alphanumeric, underscore, hyphen, 1-64 chars
    private static final Pattern GAME_ID_PATTERN = Pattern.compile("^[a-zA-Z0-9_-]{1,64}$");

    private static final String MIGO_GAMES_DIR = "migo/games";

    private final String gameId;
    private final File userDataDir;
    private final File cacheDir;
    private final File codeDir;
    private final File tempDir;

    /**
     * Create game paths from RuntimeConfig and game ID.
     *
     * @param config RuntimeConfig with base directories
     * @param gameId Unique game identifier
     * @throws IllegalArgumentException if gameId is invalid
     */
    public GamePaths(RuntimeConfig config, String gameId) {
        if (!isValidGameId(gameId)) {
            throw new IllegalArgumentException(
                "Invalid gameId: must be 1-64 alphanumeric characters, underscore or hyphen");
        }
        this.gameId = gameId;

        // Persistent storage: filesDir/migo/games/{gameId}/
        File filesBase = new File(config.getFilesDir(), MIGO_GAMES_DIR + "/" + gameId);
        this.userDataDir = new File(filesBase, "user_data");
        this.codeDir = new File(filesBase, "code");

        // Cache storage: cacheDir/migo/games/{gameId}/
        File cacheBase = new File(config.getCacheDir(), MIGO_GAMES_DIR + "/" + gameId);
        this.cacheDir = cacheBase;
        this.tempDir = new File(cacheBase, "tmp");
    }

    /**
     * Validate a game ID.
     *
     * @param gameId The game ID to validate
     * @return true if valid
     */
    public static boolean isValidGameId(String gameId) {
        return gameId != null && GAME_ID_PATTERN.matcher(gameId).matches();
    }

    /**
     * Get the game ID.
     */
    public String getGameId() {
        return gameId;
    }

    /**
     * Get the user data directory (persistent).
     * <p>
     * Use for saves, preferences, user-generated content.
     * Maps to virtual path: /user
     */
    public File getUserDataDir() {
        return userDataDir;
    }

    /**
     * Get the cache directory.
     * <p>
     * Use for downloaded resources, decoded images, etc.
     * May be cleared by the system when storage is low.
     * <p>
     * This is the per-game cache root, and the game does not see it directly:
     * it also holds runtime state — the subpackage install store and record, and
     * install staging directories — so the virtual path {@code /cache} is mapped
     * to a dedicated subdirectory of it instead. A game that could write the
     * install record would decide which package bytes a later session mounts as
     * {@code /code}.
     */
    public File getCacheDir() {
        return cacheDir;
    }

    /**
     * Get the code directory (read-only for game).
     * <p>
     * Contains the game's JavaScript code and assets.
     * After the first successful signed launch, Migo clears Unix write bits on
     * the active tree. A trusted installer must stage a complete sibling tree
     * for an update; it must serialize deployment against session start, stop
     * every session for this game, and publish only a complete tree. It must not
     * overwrite the active tree in place or assume {@link File#renameTo(File)}
     * atomically replaces a non-empty directory on API 26. Use a recoverable
     * whole-tree transaction and finish recovery before the next start. A
     * detached old tree may have its write bits restored before cleanup.
     * Maps to virtual path: /code
     */
    public File getCodeDir() {
        return codeDir;
    }

    /**
     * Get the temporary directory.
     * <p>
     * Cleared when the session ends.
     * Maps to virtual path: /tmp
     */
    public File getTempDir() {
        return tempDir;
    }

    /**
     * Ensure all directories exist.
     * Call this before starting the game.
     */
    public void ensureDirectories() {
        userDataDir.mkdirs();
        cacheDir.mkdirs();
        tempDir.mkdirs();
        // codeDir is created by download/extraction logic
    }

    /**
     * Clean up temporary files.
     * Call this when the session ends.
     */
    public void cleanupTemp() {
        deleteRecursive(tempDir);
        tempDir.mkdirs();
    }

    /**
     * Delete all game data (uninstall).
     * <p>
     * Removes both persistent and cache data.
     */
    public void deleteAll() {
        deleteRecursive(userDataDir.getParentFile()); // Delete filesDir/.../gameId/
        deleteRecursive(cacheDir);                     // Delete cacheDir/.../gameId/
    }

    /**
     * Get the total size of user data in bytes.
     */
    public long getUserDataSize() {
        return getDirectorySize(userDataDir);
    }

    /**
     * Get the total size of cache in bytes.
     */
    public long getCacheSize() {
        return getDirectorySize(cacheDir);
    }

    private static void deleteRecursive(File file) {
        if (file == null || !file.exists()) return;
        if (file.isDirectory()) {
            // Signed launch seals the active code tree. Trusted uninstall must
            // restore owner traversal/mutation bits before removing children.
            file.setWritable(true, true);
            file.setExecutable(true, true);
            File[] children = file.listFiles();
            if (children != null) {
                for (File child : children) {
                    deleteRecursive(child);
                }
            }
        }
        file.delete();
    }

    private static long getDirectorySize(File dir) {
        if (dir == null || !dir.exists()) return 0;
        if (dir.isFile()) return dir.length();

        long size = 0;
        File[] children = dir.listFiles();
        if (children != null) {
            for (File child : children) {
                size += getDirectorySize(child);
            }
        }
        return size;
    }

    @Override
    public String toString() {
        return "GamePaths{" +
                "gameId='" + gameId + '\'' +
                ", userDataDir=" + userDataDir +
                ", cacheDir=" + cacheDir +
                ", codeDir=" + codeDir +
                ", tempDir=" + tempDir +
                '}';
    }
}
