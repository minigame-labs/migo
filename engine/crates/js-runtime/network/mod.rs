use deno_core::extension;

use crate::network::fetch::{op_fetch, op_fetch_send, op_fetch_upload};
use crate::network::websocket::{op_ws_create, op_ws_next_event, op_ws_send, op_ws_close};

mod fetch;
mod websocket;

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
  ops = [
    op_fetch, op_fetch_send, op_fetch_upload,
    op_ws_create, op_ws_next_event, op_ws_send, op_ws_close,
  ],
  esm = [
     dir "network",
     "01_header.js",
     "02_response.js",
     "03_task.js",
     "04_request.js",
     "05_download.js",
     "06_upload.js",
     "07_websocket.js",
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
