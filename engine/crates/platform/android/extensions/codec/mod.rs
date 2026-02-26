use deno_core::{Extension, ToJsBuffer, extension, op2};
use deno_error::JsErrorBox;

use crate::android::jni::outbound;

/// Android-specific GBK encoding using the platform's built-in java.nio.charset.Charset via JNI.
///
/// This overrides the default GBK encoding path (Rust `encoding_rs` crate) at the JS layer.
/// Android's `encode({ format: "gbk" })` calls this op instead of `op_encode_multi_formats`,
/// eliminating the need for the Rust `encoding_rs` crate dependency on Android.
#[op2]
#[serde]
pub fn op_encode_gbk_android(#[string] data: &str) -> Result<ToJsBuffer, JsErrorBox> {
    outbound::encode_gbk(data)
        .map(ToJsBuffer::from)
        .map_err(JsErrorBox::generic)
}

/// Android-specific GBK decoding using the platform's built-in java.nio.charset.Charset via JNI.
///
/// This overrides the default GBK decoding path (Rust `encoding_rs` crate) at the JS layer.
/// Android's `decode({ format: "gbk" })` calls this op instead of `op_decode_multi_formats`,
/// eliminating the need for the Rust `encoding_rs` crate dependency on Android.
#[op2]
#[string]
pub fn op_decode_gbk_android(#[buffer] buf: &[u8]) -> Result<String, JsErrorBox> {
    outbound::decode_gbk(buf).map_err(JsErrorBox::generic)
}

extension!(host_v8_codec_android,
    ops = [op_encode_gbk_android, op_decode_gbk_android],
    esm = [
        dir "android/extensions/codec",
        "01_codec_android.js"
    ]
);

pub fn codec_extensions() -> Vec<Extension> {
    vec![host_v8_codec_android::init()]
}
