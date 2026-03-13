//! Media service ops (Camera) and ESM modules.

use deno_core::{Extension, OpState, op2};
use deno_error::JsErrorBox;
use shared::op_state::HostOpState;

// ==================== Camera Ops ====================

/// Create a camera instance with platform-specific implementation.
/// Options are passed as JSON string for extensibility.
/// Returns JSON: `{"cameraId": <id>}` on success.
#[op2]
#[string]
pub fn op_camera_create(
    state: &mut OpState,
    #[string] options_json: &str,
) -> Result<String, JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(camera) = services.camera() {
            return camera.create(options_json).map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic("createCamera:fail not supported"))
}

/// Destroy a camera instance and release all resources.
#[op2(fast)]
pub fn op_camera_destroy(
    state: &mut OpState,
    #[smi] camera_id: u32,
) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(camera) = services.camera() {
            return camera.destroy(camera_id).map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic("camera.destroy:fail not supported"))
}

/// Take a photo. Options as JSON, returns JSON with tempImagePath.
#[op2]
#[string]
pub fn op_camera_take_photo(
    state: &mut OpState,
    #[string] options_json: &str,
) -> Result<String, JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(camera) = services.camera() {
            return camera.take_photo(options_json).map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic("camera.takePhoto:fail not supported"))
}

/// Start video recording. Options as JSON.
#[op2]
#[string]
pub fn op_camera_start_record(
    state: &mut OpState,
    #[string] options_json: &str,
) -> Result<String, JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(camera) = services.camera() {
            return camera.start_record(options_json).map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic("camera.startRecord:fail not supported"))
}

/// Stop video recording. Returns JSON with tempThumbPath, tempVideoPath.
#[op2]
#[string]
pub fn op_camera_stop_record(
    state: &mut OpState,
    #[string] options_json: &str,
) -> Result<String, JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(camera) = services.camera() {
            return camera.stop_record(options_json).map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic("camera.stopRecord:fail not supported"))
}

/// Set camera zoom level. Returns JSON with actual zoom applied.
#[op2]
#[string]
pub fn op_camera_set_zoom(
    state: &mut OpState,
    #[string] options_json: &str,
) -> Result<String, JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(camera) = services.camera() {
            return camera.set_zoom(options_json).map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic("camera.setZoom:fail not supported"))
}

/// Start listening for camera frame changes (high-frequency streaming).
#[op2(fast)]
pub fn op_camera_listen_frame_change(
    state: &mut OpState,
    #[smi] camera_id: u32,
) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(camera) = services.camera() {
            return camera.listen_frame_change(camera_id).map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic("camera.listenFrameChange:fail not supported"))
}

/// Stop listening for camera frame changes.
#[op2(fast)]
pub fn op_camera_close_frame_change(
    state: &mut OpState,
    #[smi] camera_id: u32,
) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(camera) = services.camera() {
            return camera.close_frame_change(camera_id).map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic("camera.closeFrameChange:fail not supported"))
}

// ==================== Image API Ops ====================

/// Save image to system photo album.
#[op2(fast)]
pub fn op_save_image_to_photos_album(
    state: &mut OpState,
    #[string] options_json: &str,
) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(svc) = services.image_api() {
            return svc
                .save_image_to_photos_album(options_json)
                .map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic(
        "saveImageToPhotosAlbum:fail not supported",
    ))
}

/// Preview images and videos.
#[op2(fast)]
pub fn op_preview_media(
    state: &mut OpState,
    #[string] options_json: &str,
) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(svc) = services.image_api() {
            return svc.preview_media(options_json).map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic("previewMedia:fail not supported"))
}

/// Preview images fullscreen.
#[op2(fast)]
pub fn op_preview_image(
    state: &mut OpState,
    #[string] options_json: &str,
) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(svc) = services.image_api() {
            return svc.preview_image(options_json).map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic("previewImage:fail not supported"))
}

/// Compress image (async, result via callback).
#[op2(fast)]
pub fn op_compress_image(
    state: &mut OpState,
    #[string] options_json: &str,
) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(svc) = services.image_api() {
            return svc
                .compress_image(options_json)
                .map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic("compressImage:fail not supported"))
}

/// Choose files from client session (async, result via callback).
#[op2(fast)]
pub fn op_choose_message_file(
    state: &mut OpState,
    #[string] options_json: &str,
) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(svc) = services.image_api() {
            return svc
                .choose_message_file(options_json)
                .map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic("chooseMessageFile:fail not supported"))
}

/// Choose images from album or camera (async, result via callback).
#[op2(fast)]
pub fn op_choose_image(
    state: &mut OpState,
    #[string] options_json: &str,
) -> Result<(), JsErrorBox> {
    let host = state.borrow::<HostOpState>();
    if let Some(ref services) = host.device_services {
        if let Some(svc) = services.image_api() {
            return svc.choose_image(options_json).map_err(JsErrorBox::generic);
        }
    }
    Err(JsErrorBox::generic("chooseImage:fail not supported"))
}

// ==================== Extension Definition ====================

deno_core::extension!(
    host_v8_media,
    deps = [host_v8_base],
    ops = [
        op_camera_create,
        op_camera_destroy,
        op_camera_take_photo,
        op_camera_start_record,
        op_camera_stop_record,
        op_camera_set_zoom,
        op_camera_listen_frame_change,
        op_camera_close_frame_change,
        op_save_image_to_photos_album,
        op_preview_media,
        op_preview_image,
        op_compress_image,
        op_choose_message_file,
        op_choose_image,
    ],
    esm = [
        dir "media",
        "01_camera.js",
        "02_image_api.js",
    ],
);

pub fn media_extensions() -> Vec<Extension> {
    vec![host_v8_media::init()]
}

pub fn media_lazy_extensions() -> Vec<Extension> {
    vec![host_v8_media::lazy_init()]
}
