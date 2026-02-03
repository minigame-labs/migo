use deno_core::Extension;
use shared::op_state::HostOpState;

deno_core::extension!(
    host_v8_base,
    esm = [
        dir "base",
        "01_amdshim.js",
        "02_wx_async.js",
    ],
    options = {
        options: HostOpState,
    },
    state = |state, options| {
        state.put::<HostOpState>(options.options);
    },
);

pub fn base_extensions(host: HostOpState) -> Vec<Extension> {
    vec![host_v8_base::init_ops_and_esm(host)]
}
