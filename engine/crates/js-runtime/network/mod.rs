use deno_core::extension;

use crate::network::fetch::{op_fetch, op_fetch_send};

mod fetch;

#[derive(Clone)]
struct Options {
    pub user_agent: String,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            user_agent: "migo".to_string(),
        }
    }
}

extension!(host_v8_network,
  deps = [host_v8_console, host_v8_web, host_v8_base],
  ops = [op_fetch, op_fetch_send],
  esm = [
     dir "network",
     "24_header.js",
     "25_request_task.js",
     "25_response.js",
     "26_request.js"
  ],
  options = {
    options: Options,
  },
  state = |state, options| {
    state.put::<Options>(options.options);
  },
);


pub(crate) fn network_extensions() -> Vec<deno_core::Extension> {
    vec![host_v8_network::init_ops_and_esm(Default::default())]
}
