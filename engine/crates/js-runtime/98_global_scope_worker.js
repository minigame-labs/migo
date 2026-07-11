// Worker-specific global scope.
// Bare-imports all ESM modules from extensions loaded by the worker runtime
// that are NOT already imported by 98_global_scope_shared.js or 02_worker_inner.js.
// This ensures every extension ESM module is evaluated (deno_core requirement).

// console
import "ext:host_v8_console/01_alert.js";

// base
import "ext:host_v8_base/02_async.js";
import "ext:host_v8_base/04_subpackage.js";

// lifecycle
import "ext:host_v8_lifecycle/02_restart_exit.js";

// url
import "ext:host_v8_url/03_url.js";

// web
import "ext:host_v8_web/12_performance.js";
import "ext:host_v8_web/03_canvas.js";

// network
import "ext:host_v8_network/08_tcp_socket.js";
import "ext:host_v8_network/09_udp_socket.js";

// webgl (registered by rendering extension)
import "ext:host_v8_webgl/01_constants.js";
import "ext:host_v8_webgl/02_2d_context.js";
import "ext:host_v8_webgl/02_webgl_context.js";
import "ext:host_v8_webgl/03_raf.js";
import "ext:host_v8_webgl/04_font.js";

// audio (registered by audio extension)
import "ext:host_v8_audio/00_audio_param.js";
import "ext:host_v8_audio/00_audio_buffer.js";
import "ext:host_v8_audio/00_audio_node.js";
import "ext:host_v8_audio/00_buffer_source_node.js";
import "ext:host_v8_audio/00_gain_node.js";
import "ext:host_v8_audio/00_oscillator_node.js";
import "ext:host_v8_audio/00_delay_node.js";
import "ext:host_v8_audio/00_biquad_filter_node.js";
import "ext:host_v8_audio/00_wave_shaper_node.js";
import "ext:host_v8_audio/00_analyser_node.js";
import "ext:host_v8_audio/00_dynamics_compressor_node.js";
import "ext:host_v8_audio/00_panner_node.js";
import "ext:host_v8_audio/00_channel_merger_node.js";
import "ext:host_v8_audio/00_channel_splitter_node.js";
import "ext:host_v8_audio/00_constant_source_node.js";
import "ext:host_v8_audio/00_iir_filter_node.js";
import "ext:host_v8_audio/00_script_processor_node.js";
import "ext:host_v8_audio/00_periodic_wave.js";
import "ext:host_v8_audio/00_audio_listener.js";
import "ext:host_v8_audio/01_audio_context.js";
import "ext:host_v8_audio/03_audio_interruption.js";
import "ext:host_v8_audio/04_media_audio_player.js";
import "ext:host_v8_audio/05_recorder_manager.js";
