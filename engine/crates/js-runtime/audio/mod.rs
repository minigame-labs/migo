use deno_core::{Extension, extension};

use crate::audio::ops::{
    op_audio_close_context, op_audio_connect, op_audio_create_buffer_source,
    op_audio_create_context, op_audio_create_gain, op_audio_decode_audio_data,
    op_audio_disconnect, op_audio_set_buffer, op_audio_set_gain_value, op_audio_set_loop,
    op_audio_start, op_audio_stop,
    // InnerAudioContext ops
    op_inner_audio_create, op_inner_audio_destroy, op_inner_audio_load, op_inner_audio_load_url,
    op_inner_audio_play, op_inner_audio_pause, op_inner_audio_stop,
    op_inner_audio_seek, op_inner_audio_set_volume, op_inner_audio_set_loop,
    op_inner_audio_set_playback_rate, op_inner_audio_set_autoplay, op_inner_audio_get_state,
};

mod ops;

extension!(host_v8_audio,
    deps = [host_v8_base],
    ops = [
        // WebAudio ops
        op_audio_create_context,
        op_audio_close_context,
        op_audio_decode_audio_data,
        op_audio_create_buffer_source,
        op_audio_set_buffer,
        op_audio_start,
        op_audio_stop,
        op_audio_set_loop,
        op_audio_create_gain,
        op_audio_set_gain_value,
        op_audio_connect,
        op_audio_disconnect,
        // InnerAudioContext ops
        op_inner_audio_create,
        op_inner_audio_destroy,
        op_inner_audio_load,
        op_inner_audio_load_url,
        op_inner_audio_play,
        op_inner_audio_pause,
        op_inner_audio_stop,
        op_inner_audio_seek,
        op_inner_audio_set_volume,
        op_inner_audio_set_loop,
        op_inner_audio_set_playback_rate,
        op_inner_audio_set_autoplay,
        op_inner_audio_get_state,
    ],
    esm = [
        dir "audio",
        "00_audio_param.js",
        "00_audio_buffer.js",
        "00_audio_node.js",
        "00_buffer_source_node.js",
        "00_gain_node.js",
        "01_audio_context.js",
        "02_inner_audio_context.js",
        "03_audio_interruption.js",
    ],
);

pub fn audio_extensions() -> Vec<Extension> {
    vec![host_v8_audio::init()]
}
