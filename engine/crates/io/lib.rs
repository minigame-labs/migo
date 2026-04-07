mod fast_image_decoder;
mod image_cache;
mod io_cmd_handler;
mod io_thread;
pub mod ktx2;
#[cfg(feature = "zip-extract")]
mod zip_extract;
pub mod derived_cache;
#[cfg(feature = "zip-extract")]
pub mod package_ingest;

pub use fast_image_decoder::{
    CompressedImageInfo, decode_image_fast, detect_compressed_format, register_platform_decoder,
};
pub use image_cache::{CacheStats, CachedImage, ImageCache, global_cache};
pub use io_thread::run_io_handler;
#[cfg(feature = "zip-extract")]
pub use zip_extract::{ZipError, extract_zip, extract_zip_async};
#[cfg(feature = "zip-extract")]
pub use package_ingest::ingest_zip_to_package;
