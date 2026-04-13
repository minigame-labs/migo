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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PriorityClass {
    /// Lowest priority: unzip, ingest, preloading, warming.
    Background,
    /// Runtime on-demand async loading.
    ForegroundAsync,
    /// Sync APIs and startup-critical loads blocking current execution.
    ForegroundBlocking,
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

impl IoRequest {
    pub fn priority(&self) -> PriorityClass {
        match self {
            IoRequest::ReadFile { priority, .. }
            | IoRequest::GetFileInfo { priority, .. }
            | IoRequest::DecodeImage { priority, .. }
            | IoRequest::Unzip { priority, .. }
            | IoRequest::PackageIngest { priority, .. }
            | IoRequest::StorageGet { priority, .. }
            | IoRequest::StorageMutate { priority, .. }
            | IoRequest::StorageInfo { priority, .. } => *priority,
        }
    }
}

impl From<RequestKind> for PriorityClass {
    fn from(kind: RequestKind) -> Self {
        match kind {
            RequestKind::Sync => PriorityClass::ForegroundBlocking,
            RequestKind::Async => PriorityClass::ForegroundAsync,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_request_priority_extracts_correct_class() {
        let read = IoRequest::ReadFile {
            backend: BackendKind::Pack,
            request: RequestKind::Sync,
            priority: PriorityClass::ForegroundBlocking,
            spec: ReadSpec::Whole,
            estimated_bytes: 0,
        };
        assert_eq!(read.priority(), PriorityClass::ForegroundBlocking);

        let unzip = IoRequest::Unzip {
            backend: BackendKind::Archive,
            priority: PriorityClass::Background,
            compressed_bytes: 0,
        };
        assert_eq!(unzip.priority(), PriorityClass::Background);

        let storage = IoRequest::StorageMutate {
            request: RequestKind::Async,
            priority: PriorityClass::ForegroundAsync,
        };
        assert_eq!(storage.priority(), PriorityClass::ForegroundAsync);
    }

    #[test]
    fn priority_class_ordering() {
        assert!(PriorityClass::ForegroundBlocking > PriorityClass::ForegroundAsync);
        assert!(PriorityClass::ForegroundAsync > PriorityClass::Background);
        assert!(PriorityClass::ForegroundBlocking > PriorityClass::Background);
    }
}
