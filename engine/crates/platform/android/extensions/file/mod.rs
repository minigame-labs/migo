use deno_core::{Extension, extension, op2};
use deno_error::JsErrorBox;

use crate::android::jni::outbound;

/// Android-specific unzip using the platform's built-in java.util.zip via JNI.
///
/// This overrides the default `op_unzip` (Rust zip crate) at the JS layer.
/// Android's `FileManager.unzip()` calls this op instead of the base `op_unzip`,
/// eliminating the need for the Rust `zip` crate dependency on Android.
#[op2(async(lazy), fast)]
pub async fn op_unzip_android(
    #[string] zip_file_path: String,
    #[string] target_path: String,
) -> Result<(), JsErrorBox> {
    tokio::task::spawn_blocking(move || -> Result<usize, String> {
        outbound::unzip_file(&zip_file_path, &target_path)
    })
    .await
    .map_err(|e| JsErrorBox::generic(format!("unzip task join error: {e}")))?
    .map(|_| ()) // Discard file count, JS doesn't need it
    .map_err(|e| JsErrorBox::generic(e))
}

extension!(host_v8_file_android,
 ops=[op_unzip_android],
 esm = [
    dir "android/extensions/file",
    "01_file_manager.js"
 ]
);

pub fn file_extensions() -> Vec<Extension> {
    vec![host_v8_file_android::init()]
}
