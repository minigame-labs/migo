//! Pass 2 of the GL command stream, as this runtime reaches it.
//!
//! The decoder itself is in `frame-decode`, which links no JavaScript engine:
//! turning validated words into render commands is engine-neutral work, and
//! the Apple Performance+ product needs exactly this with no V8 anywhere in
//! its dependency closure. It used to live here, which is what made that
//! product impossible.
//!
//! What is left is the adapter. `frame-decode` needs somewhere to push a WebGL
//! error and one piece of GL state; here that is the op state's
//! `WebGLErrorState`, and `OpStateDecodeContext` is the two-method bridge.

use deno_core::OpState;
use shared::protocol::render_cmd::GLCmd;

use crate::rendering::webgl::{error_state::OpStateDecodeContext, gl_stream::ValidatedStream};

/// Decode a structurally-validated GL command stream into owned `GLCmd` values.
///
/// Returns the saturating approximate byte count for all accepted commands.
pub(crate) fn decode_validated_stream(
    state: &mut OpState,
    stream: ValidatedStream<'_>,
    out: &mut Vec<GLCmd>,
) -> usize {
    frame_decode::decode_validated_stream(&mut OpStateDecodeContext(state), stream, out)
}
