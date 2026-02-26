mod fast_image_decoder;
mod image_cache;
mod io_cmd_handler;
mod io_thread;
#[cfg(feature = "zip-extract")]
mod zip_extract;

pub use fast_image_decoder::decode_image_fast;
pub use image_cache::{global_cache, CacheStats, CachedImage, ImageCache};
pub use io_thread::*;
#[cfg(feature = "zip-extract")]
pub use zip_extract::{extract_zip, extract_zip_async, ZipError};