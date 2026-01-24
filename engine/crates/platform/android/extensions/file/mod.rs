use deno_core::{Extension, OpState, extension, op2};
use deno_error::JsErrorBox;
use shared::op_state::HostOpState;

use crate::android::jni::*;

#[op2(fast)]
pub fn op_unzip(
    state: &mut OpState,
    #[smi] request_id: i32,
    #[string] zip_file_path: String,
    #[string] target_path: String,
) -> Result<(), JsErrorBox> {
    let options = state.borrow::<HostOpState>();
    unzip(options.id, request_id, &zip_file_path, &target_path)
        .map_err(|e| JsErrorBox::generic(e.to_string()))?;
    Ok(())
}

extension!(host_v8_file_android,
 ops=[op_unzip],
 esm = [
    dir "android/extensions/file",
    "01_file_manager.js"
 ]
);

pub fn file_extensions() -> Vec<Extension> {
    vec![host_v8_file_android::init_ops_and_esm()]
}
