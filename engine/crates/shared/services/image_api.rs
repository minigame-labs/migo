//! Image API service traits for image picking, preview, save, and compression.

use crate::protocol::error::ServiceError;

/// Image API service for image-related operations.
pub trait ImageApiService: Send + Sync {
    /// Save an image to the system photo album.
    ///
    /// JSON fields (input):
    /// - `filePath`: string — local file path of the image
    fn save_image_to_photos_album(&self, _options_json: &str) -> Result<(), ServiceError> {
        Err(ServiceError::not_supported(
            "saveImageToPhotosAlbum:fail not supported",
        ))
    }

    /// Preview images and videos in a fullscreen viewer.
    ///
    /// JSON fields (input):
    /// - `sources`: array of `{url, type, poster}` — resources to preview
    /// - `current`: number — index of current resource (default 0)
    /// - `showmenu`: boolean — show long-press menu (default true)
    fn preview_media(&self, _options_json: &str) -> Result<(), ServiceError> {
        Err(ServiceError::not_supported(
            "previewMedia:fail not supported",
        ))
    }

    /// Preview images in a fullscreen viewer.
    ///
    /// JSON fields (input):
    /// - `urls`: array of string — image URLs
    /// - `current`: string — current image URL
    /// - `showmenu`: boolean — show long-press menu (default true)
    fn preview_image(&self, _options_json: &str) -> Result<(), ServiceError> {
        Err(ServiceError::not_supported(
            "previewImage:fail not supported",
        ))
    }

    /// Compress an image (async, result via callback).
    ///
    /// JSON fields (input):
    /// - `src`: string — image file path
    /// - `quality`: number — compression quality 0-100 (default 80)
    /// - `compressedWidth`: number (optional)
    /// - `compressedHeight`: number (optional)
    ///
    /// Result delivered via `onCompressImageResult` callback with JSON: `{"tempFilePath": "..."}`
    fn compress_image(&self, _options_json: &str) -> Result<(), ServiceError> {
        Err(ServiceError::not_supported(
            "compressImage:fail not supported",
        ))
    }

    /// Choose files from client session (async, result via callback).
    ///
    /// JSON fields (input):
    /// - `count`: number — max files to select (0-100)
    /// - `type`: string — "all", "video", "image", "file"
    /// - `extension`: array of string (only when type=="file")
    fn choose_message_file(&self, _options_json: &str) -> Result<(), ServiceError> {
        Err(ServiceError::not_supported(
            "chooseMessageFile:fail not supported",
        ))
    }

    /// Choose images from album or camera (async, result via callback).
    ///
    /// JSON fields (input):
    /// - `count`: number — max images (default 9)
    /// - `sizeType`: array of string — ["original", "compressed"]
    /// - `sourceType`: array of string — ["album", "camera"]
    fn choose_image(&self, _options_json: &str) -> Result<(), ServiceError> {
        Err(ServiceError::not_supported(
            "chooseImage:fail not supported",
        ))
    }
}
