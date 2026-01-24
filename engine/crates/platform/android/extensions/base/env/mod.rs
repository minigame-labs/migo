use deno_core::{Extension, OpState, extension, op2};
use shared::op_state::HostOpState;

#[op2]
#[string]
pub fn op_get_user_data_path(state: &mut OpState) -> Option<String> {
    let options = state.borrow::<HostOpState>();
    options.app_tmp_dir.to_str().map(|s| s.to_string())
}

extension!(host_v8_env,
 ops = [op_get_user_data_path],
 esm = [
    dir "android/extensions/base/env",
    "00_env.js"
 ]
);

pub fn env_extensions() -> Vec<Extension> {
    vec![host_v8_env::init_ops_and_esm()]
}
