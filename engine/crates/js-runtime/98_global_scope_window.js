import * as alert from "ext:host_v8_console/01_alert.js";
import * as url from "ext:host_v8_url/03_url.js";
import * as globalInterfaces from "ext:host_v8_web/04_global_interfaces.js";
import * as location from "ext:host_v8_web/12_location.js";
import * as performance from "ext:host_v8_web/12_performance.js";
import * as raf from "ext:host_v8_webgl/03_raf.js";
import * as canvas from "ext:host_v8_web/03_canvas.js";
import * as webgl from 'ext:host_v8_webgl/02_webgl_context.js';
import * as touch from 'ext:host_v8_touch/01_touch.js';
import * as audio from 'ext:host_v8_audio/01_audio_context.js';
import * as innerAudio from 'ext:host_v8_audio/02_inner_audio_context.js';
import * as audioInterruption from 'ext:host_v8_audio/03_audio_interruption.js';

import { core } from "ext:core/mod.js";

const WindowGlobalScope = {
    alert: core.propWritable(alert.alert),
    URL: core.propNonEnumerable(url.URL),
    location: core.propNonEnumerable(location.location),
    Window: core.propNonEnumerable(globalInterfaces.Window),
    Canvas: core.propNonEnumerable(canvas.Canvas),
    createCanvas: core.propNonEnumerable(canvas.createCanvas),
    requestAnimationFrame: core.propWritable(raf.requestAnimationFrame),
    cancelAnimationFrame: core.propWritable(raf.cancelAnimationFrame),
    _internalScheduleRaf: core.propNonEnumerable(raf._internalScheduleRaf),
    
    WebGLRenderingContext: core.propNonEnumerable(webgl.WebGLRenderingContext),
    WebGL2RenderingContext: core.propNonEnumerable(webgl.WebGL2RenderingContext),

    performance: core.propNonEnumerable(performance.performance),

    // Touch
    onTouchStart: core.propNonEnumerable(touch.onTouchStart),
    onTouchMove: core.propNonEnumerable(touch.onTouchMove),
    onTouchEnd: core.propNonEnumerable(touch.onTouchEnd),
    onTouchCancel: core.propNonEnumerable(touch.onTouchCancel),
    offTouchMove: core.propNonEnumerable(touch.offTouchMove),
    offTouchEnd: core.propNonEnumerable(touch.offTouchEnd),
    offTouchCancel: core.propNonEnumerable(touch.offTouchCancel),
    _internalEnqueueRawTouchEvent: core.propNonEnumerable(touch._internalEnqueueRawTouchEvent),

    // Audio (WebAudio API)
    AudioContext: core.propNonEnumerable(audio.AudioContext),
    AudioBuffer: core.propNonEnumerable(audio.AudioBuffer),
    AudioNode: core.propNonEnumerable(audio.AudioNode),
    AudioDestinationNode: core.propNonEnumerable(audio.AudioDestinationNode),
    AudioBufferSourceNode: core.propNonEnumerable(audio.AudioBufferSourceNode),
    GainNode: core.propNonEnumerable(audio.GainNode),
    AudioParam: core.propNonEnumerable(audio.AudioParam),

    // InnerAudioContext
    InnerAudioContext: core.propNonEnumerable(innerAudio.InnerAudioContext),
    createInnerAudioContext: core.propNonEnumerable(innerAudio.createInnerAudioContext),
    _internalEnqueueInnerAudioEvent: core.propNonEnumerable(innerAudio._internalEnqueueInnerAudioEvent),

    // Audio Interruption
    onAudioInterruptionBegin: core.propNonEnumerable(audioInterruption.onAudioInterruptionBegin),
    offAudioInterruptionBegin: core.propNonEnumerable(audioInterruption.offAudioInterruptionBegin),
    onAudioInterruptionEnd: core.propNonEnumerable(audioInterruption.onAudioInterruptionEnd),
    offAudioInterruptionEnd: core.propNonEnumerable(audioInterruption.offAudioInterruptionEnd),
    _internalTriggerAudioInterruptionBegin: core.propNonEnumerable(audioInterruption._internalTriggerAudioInterruptionBegin),
    _internalTriggerAudioInterruptionEnd: core.propNonEnumerable(audioInterruption._internalTriggerAudioInterruptionEnd),
};

export { WindowGlobalScope };