use deno_core::v8;
use deno_core::{OpState, op2};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// TODO: remove this
pub type CallbackMap = Arc<Mutex<HashMap<u32, v8::Global<v8::Function>>>>;

#[op2]
pub(crate) fn op_register_raf_callback(
    state: &mut OpState,
    #[smi] callback_id: u32,
    #[global] callback: v8::Global<v8::Function>,
) {
    let cb_map = state.borrow::<CallbackMap>();
    cb_map.lock().unwrap().insert(callback_id, callback);
}

#[op2(fast)]
pub(crate) fn op_cancel_raf_callback(state: &mut OpState, #[smi] callback_id: u32) {
    let map = state.borrow::<CallbackMap>();
    map.lock().unwrap().remove(&callback_id);
}
