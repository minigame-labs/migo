/**
 * Environment constants for game code.
 * 
 * All paths are virtual and sandboxed to the game's isolated directories:
 * - /user - User data (saves, preferences) - persistent, read/write
 * - /cache - Cache files - may be cleared by system, read/write
 * - /code - Game code directory - read only
 * - /tmp - Temporary files - cleared on session end, read/write
 */
const env = {
    /** User data directory for saves and preferences (virtual path) */
    USER_DATA_PATH: "/user",
    
    /** Cache directory for temporary files (virtual path) */
    CACHE_PATH: "/cache",
    
    /** Game code directory (virtual path, read-only) */
    CODE_PATH: "/code",
    
    /** Temporary files directory (virtual path) */
    TEMP_PATH: "/tmp"
};

export { env };
