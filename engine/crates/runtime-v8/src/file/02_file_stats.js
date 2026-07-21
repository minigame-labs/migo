class Stats {
    constructor(stat = {}) {
        const mode = stat.mode ?? 0;
        const size = stat.size ?? 0;

        // Rust side currently sends seconds since UNIX_EPOCH
        const atimeSec = stat.atime ?? 0;
        const mtimeSec = stat.mtime ?? 0;

        this.mode = mode;
        this.size = size;

        // Keep original fields for backward compatibility (seconds)
        this.lastAccessedTime = atimeSec;
        this.lastModifiedTime = mtimeSec;

        // Add ms + Date helpers (non-breaking)
        this.lastAccessedTimeMs = atimeSec * 1000;
        this.lastModifiedTimeMs = mtimeSec * 1000;
        this.lastAccessedDate = new Date(this.lastAccessedTimeMs);
        this.lastModifiedDate = new Date(this.lastModifiedTimeMs);

        this._isFile = !!stat.is_file;
        this._isDirectory = !!stat.is_directory;
    }

    isFile() {
        return this._isFile;
    }

    isDirectory() {
        return this._isDirectory;
    }
}

class FileStats {
    constructor(path, stat) {
        this.stats = new Stats(stat);
        this.path = path;
    }
}

export { Stats, FileStats };
