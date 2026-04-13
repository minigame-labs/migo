use std::sync::Arc;

use deno_core::Extension;
use shared::op_state::HostOpState;

pub(crate) struct IoSchedulerState(pub Arc<io::scheduler::IoScheduler>);

deno_core::extension!(
    host_v8_io_state,
    deps = [host_v8_base],
    state = |state| {
        let host_id = state.borrow::<HostOpState>().id;
        state.put(IoSchedulerState(Arc::new(io::scheduler::IoScheduler::new(
            host_id,
        ))));
    },
);

pub(crate) fn io_state_extensions() -> Vec<Extension> {
    vec![host_v8_io_state::init()]
}
