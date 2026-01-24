use deno_core::{OpState, ToJsBuffer, op2};
use deno_error::JsErrorBox;
use shared::codec;

#[inline]
fn normalize_coding<'a>(coding: &'a str) -> &'a str {
    if coding.is_empty() { "utf8" } else { coding }
}

#[op2]
#[serde]
pub(super) fn op_encode_multi_formats(
    _state: &OpState,
    #[string] original: &str,
    #[string] coding: &str,
) -> Result<ToJsBuffer, JsErrorBox> {
    let coding = normalize_coding(coding);
    codec::encode_string(original, coding)
        .map(ToJsBuffer::from)
        .map_err(JsErrorBox::generic)
}

#[op2]
#[string]
pub(super) fn op_decode_multi_formats(
    _state: &OpState,
    #[buffer] buf: &[u8],
    #[string] coding: &str,
) -> Result<String, JsErrorBox> {
    let coding = normalize_coding(coding);
    codec::decode_bytes(buf, coding).map_err(JsErrorBox::generic)
}
