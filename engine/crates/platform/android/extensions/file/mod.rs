use std::{cell::RefCell, rc::Rc};

use deno_core::{Extension, OpState, extension, op2};
use deno_error::JsErrorBox;
use shared::{
    op_state::HostOpState,
    protocol::{
        self,
        io_cmd::{IOCmd, IOCmdResp},
    },
};

/// Native unzip operation using Rust's `zip` crate.
/// Executes on the IO thread for unified IO scheduling.
#[op2(async(lazy), fast)]
pub async fn op_unzip(
    state: Rc<RefCell<OpState>>,
    #[string] zip_file_path: String,
    #[string] target_path: String,
) -> Result<(), JsErrorBox> {
    let io_tx = {
        let st = state.borrow();
        st.borrow::<HostOpState>().io_tx.clone()
    };

    protocol::send_fs_with_resp_async(&io_tx, move |resp_tx| IOCmd::Unzip {
        zip_path: zip_file_path,
        dest_dir: target_path,
        resp: IOCmdResp::Async(resp_tx),
    })
    .await
    .map(|_| ()) // Discard file count, JS doesn't need it
    .map_err(|e| JsErrorBox::generic(e.to_string()))
}

extension!(host_v8_file_android,
 ops=[op_unzip],
 esm = [
    dir "android/extensions/file",
    "01_file_manager.js"
 ]
);

pub fn file_extensions() -> Vec<Extension> {
    vec![host_v8_file_android::init()]
}
