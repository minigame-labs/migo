#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    Filesystem,
    Pack,
    Archive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKind {
    Sync,
    Async,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriorityClass {
    ForegroundBlocking,
    ForegroundAsync,
    Background,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolKind {
    Fs,
    Pack,
    Image,
    Archive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadSpec {
    Whole,
    Range { position: u64, length: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IoRequest {
    ReadFile {
        backend: BackendKind,
        request: RequestKind,
        priority: PriorityClass,
        spec: ReadSpec,
        estimated_bytes: usize,
    },
    GetFileInfo {
        backend: BackendKind,
        request: RequestKind,
        priority: PriorityClass,
    },
    DecodeImage {
        backend: BackendKind,
        request: RequestKind,
        priority: PriorityClass,
        encoded_bytes: usize,
        cache_hit: bool,
    },
    Unzip {
        backend: BackendKind,
        priority: PriorityClass,
        compressed_bytes: usize,
    },
    PackageIngest {
        priority: PriorityClass,
        compressed_bytes: usize,
    },
    StorageGet {
        request: RequestKind,
        priority: PriorityClass,
        estimated_bytes: usize,
    },
    StorageMutate {
        request: RequestKind,
        priority: PriorityClass,
    },
    StorageInfo {
        request: RequestKind,
        priority: PriorityClass,
    },
}

impl From<RequestKind> for PriorityClass {
    fn from(kind: RequestKind) -> Self {
        match kind {
            RequestKind::Sync => PriorityClass::ForegroundBlocking,
            RequestKind::Async => PriorityClass::ForegroundAsync,
        }
    }
}
