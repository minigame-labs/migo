import * as alert from "ext:host_v8_console/01_alert.js";
import * as url from "ext:host_v8_url/03_url.js";
import * as location from "ext:host_v8_web/12_location.js";
import * as performance from "ext:host_v8_web/12_performance.js";
import * as raf from "ext:host_v8_webgl/03_raf.js";
import * as fontApi from "ext:host_v8_webgl/04_font.js";
import * as canvas from "ext:host_v8_web/03_canvas.js";
import * as webgl from 'ext:host_v8_webgl/02_webgl_context.js';
import * as touch from 'ext:host_v8_touch/01_touch.js';
import * as audio from 'ext:host_v8_audio/01_audio_context.js';
import * as innerAudio from 'ext:host_v8_audio/02_inner_audio_context.js';
import * as audioInterruption from 'ext:host_v8_audio/03_audio_interruption.js';
import * as mediaAudioPlayer from 'ext:host_v8_audio/04_media_audio_player.js';
import * as recorderManager from 'ext:host_v8_audio/05_recorder_manager.js';
import * as deviceMotion from 'ext:host_v8_device/01_device_motion.js';
import * as gyroscope from 'ext:host_v8_device/02_gyroscope.js';
import * as orientation from 'ext:host_v8_device/03_orientation.js';
import * as compass from 'ext:host_v8_device/04_compass.js';
import * as accelerometer from 'ext:host_v8_device/05_accelerometer.js';
import * as battery from 'ext:host_v8_device/06_battery.js';
import * as clipboard from 'ext:host_v8_device/07_clipboard.js';
import * as vibrate from 'ext:host_v8_device/08_vibrate.js';
import * as screen from 'ext:host_v8_device/09_screen.js';
import * as network from 'ext:host_v8_device/10_network.js';
import * as workerApi from 'ext:host_v8_worker/01_worker.js';

import { core } from "ext:core/mod.js";

const WindowGlobalScope = {
    alert: core.propWritable(alert.alert),
    URL: core.propNonEnumerable(url.URL),
    location: core.propNonEnumerable(location.location),
    createCanvas: core.propNonEnumerable(canvas.createCanvas),
    requestAnimationFrame: core.propWritable(raf.requestAnimationFrame),
    cancelAnimationFrame: core.propWritable(raf.cancelAnimationFrame),
    setPreferredFramesPerSecond: core.propWritable(raf.setPreferredFramesPerSecond),

    WebGLRenderingContext: core.propNonEnumerable(webgl.WebGLRenderingContext),
    WebGL2RenderingContext: core.propNonEnumerable(webgl.WebGL2RenderingContext),

    performance: core.propNonEnumerable(performance.performance),

    // Touch
    onTouchStart: core.propNonEnumerable(touch.onTouchStart),
    onTouchMove: core.propNonEnumerable(touch.onTouchMove),
    onTouchEnd: core.propNonEnumerable(touch.onTouchEnd),
    onTouchCancel: core.propNonEnumerable(touch.onTouchCancel),
    offTouchStart: core.propNonEnumerable(touch.offTouchStart),
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
    OscillatorNode: core.propNonEnumerable(audio.OscillatorNode),
    DelayNode: core.propNonEnumerable(audio.DelayNode),
    BiquadFilterNode: core.propNonEnumerable(audio.BiquadFilterNode),
    WaveShaperNode: core.propNonEnumerable(audio.WaveShaperNode),
    AnalyserNode: core.propNonEnumerable(audio.AnalyserNode),
    DynamicsCompressorNode: core.propNonEnumerable(audio.DynamicsCompressorNode),
    PannerNode: core.propNonEnumerable(audio.PannerNode),
    ChannelMergerNode: core.propNonEnumerable(audio.ChannelMergerNode),
    ChannelSplitterNode: core.propNonEnumerable(audio.ChannelSplitterNode),
    ConstantSourceNode: core.propNonEnumerable(audio.ConstantSourceNode),
    IIRFilterNode: core.propNonEnumerable(audio.IIRFilterNode),
    ScriptProcessorNode: core.propNonEnumerable(audio.ScriptProcessorNode),
    PeriodicWave: core.propNonEnumerable(audio.PeriodicWave),
    AudioListener: core.propNonEnumerable(audio.AudioListener),
    createWebAudioContext: core.propNonEnumerable(audio.createWebAudioContext),

    // InnerAudioContext
    InnerAudioContext: core.propNonEnumerable(innerAudio.InnerAudioContext),
    createInnerAudioContext: core.propNonEnumerable(innerAudio.createInnerAudioContext),
    setInnerAudioOption: core.propNonEnumerable(innerAudio.setInnerAudioOption),
    getAvailableAudioSources: core.propNonEnumerable(innerAudio.getAvailableAudioSources),
    _internalEnqueueInnerAudioEvent: core.propNonEnumerable(innerAudio._internalEnqueueInnerAudioEvent),

    // MediaAudioPlayer
    MediaAudioPlayer: core.propNonEnumerable(mediaAudioPlayer.MediaAudioPlayer),
    createMediaAudioPlayer: core.propNonEnumerable(mediaAudioPlayer.createMediaAudioPlayer),

    // RecorderManager
    getRecorderManager: core.propNonEnumerable(recorderManager.getRecorderManager),
    _internalOnRecorderEvent: core.propNonEnumerable(recorderManager._internalOnRecorderEvent),
    _internalOnRecorderFrameData: core.propNonEnumerable(recorderManager._internalOnRecorderFrameData),

    // Audio Interruption
    onAudioInterruptionBegin: core.propNonEnumerable(audioInterruption.onAudioInterruptionBegin),
    offAudioInterruptionBegin: core.propNonEnumerable(audioInterruption.offAudioInterruptionBegin),
    onAudioInterruptionEnd: core.propNonEnumerable(audioInterruption.onAudioInterruptionEnd),
    offAudioInterruptionEnd: core.propNonEnumerable(audioInterruption.offAudioInterruptionEnd),
    _internalTriggerAudioInterruptionBegin: core.propNonEnumerable(audioInterruption._internalTriggerAudioInterruptionBegin),
    _internalTriggerAudioInterruptionEnd: core.propNonEnumerable(audioInterruption._internalTriggerAudioInterruptionEnd),

    // Device Motion
    onDeviceMotionChange: core.propNonEnumerable(deviceMotion.onDeviceMotionChange),
    offDeviceMotionChange: core.propNonEnumerable(deviceMotion.offDeviceMotionChange),
    _internalTriggerDeviceMotionChange: core.propNonEnumerable(deviceMotion._internalTriggerDeviceMotionChange),
    startDeviceMotionListening: core.propNonEnumerable(deviceMotion.startDeviceMotionListening),
    stopDeviceMotionListening: core.propNonEnumerable(deviceMotion.stopDeviceMotionListening),

    // Gyroscope
    onGyroscopeChange: core.propNonEnumerable(gyroscope.onGyroscopeChange),
    offGyroscopeChange: core.propNonEnumerable(gyroscope.offGyroscopeChange),
    _internalTriggerGyroscopeChange: core.propNonEnumerable(gyroscope._internalTriggerGyroscopeChange),
    startGyroscope: core.propNonEnumerable(gyroscope.startGyroscope),
    stopGyroscope: core.propNonEnumerable(gyroscope.stopGyroscope),

    // Device Orientation
    onDeviceOrientationChange: core.propNonEnumerable(orientation.onDeviceOrientationChange),
    offDeviceOrientationChange: core.propNonEnumerable(orientation.offDeviceOrientationChange),
    _internalTriggerDeviceOrientationChange: core.propNonEnumerable(orientation._internalTriggerDeviceOrientationChange),

    // Compass
    onCompassChange: core.propNonEnumerable(compass.onCompassChange),
    offCompassChange: core.propNonEnumerable(compass.offCompassChange),
    _internalTriggerCompassChange: core.propNonEnumerable(compass._internalTriggerCompassChange),
    startCompass: core.propNonEnumerable(compass.startCompass),
    stopCompass: core.propNonEnumerable(compass.stopCompass),

    // Accelerometer
    onAccelerometerChange: core.propNonEnumerable(accelerometer.onAccelerometerChange),
    offAccelerometerChange: core.propNonEnumerable(accelerometer.offAccelerometerChange),
    _internalTriggerAccelerometerChange: core.propNonEnumerable(accelerometer._internalTriggerAccelerometerChange),
    startAccelerometer: core.propNonEnumerable(accelerometer.startAccelerometer),
    stopAccelerometer: core.propNonEnumerable(accelerometer.stopAccelerometer),

    // Battery
    getBatteryInfo: core.propNonEnumerable(battery.getBatteryInfo),
    getBatteryInfoSync: core.propNonEnumerable(battery.getBatteryInfoSync),

    // Clipboard
    setClipboardData: core.propNonEnumerable(clipboard.setClipboardData),
    getClipboardData: core.propNonEnumerable(clipboard.getClipboardData),

    // Vibration
    vibrateShort: core.propNonEnumerable(vibrate.vibrateShort),
    vibrateLong: core.propNonEnumerable(vibrate.vibrateLong),

    // Screen
    getScreenBrightness: core.propNonEnumerable(screen.getScreenBrightness),
    setScreenBrightness: core.propNonEnumerable(screen.setScreenBrightness),
    setKeepScreenOn: core.propNonEnumerable(screen.setKeepScreenOn),
    setDeviceOrientation: core.propNonEnumerable(screen.setDeviceOrientation),

    // Font
    loadFont: core.propNonEnumerable(fontApi.loadFont),
    getTextLineHeight: core.propNonEnumerable(fontApi.getTextLineHeight),

    // Network
    onNetworkStatusChange: core.propNonEnumerable(network.onNetworkStatusChange),
    offNetworkStatusChange: core.propNonEnumerable(network.offNetworkStatusChange),
    onNetworkWeakChange: core.propNonEnumerable(network.onNetworkWeakChange),
    offNetworkWeakChange: core.propNonEnumerable(network.offNetworkWeakChange),
    _internalTriggerNetworkStatusChange: core.propNonEnumerable(network._internalTriggerNetworkStatusChange),
    getNetworkType: core.propNonEnumerable(network.getNetworkType),
    getLocalIPAddress: core.propNonEnumerable(network.getLocalIPAddress),

    // Worker
    createWorker: core.propNonEnumerable(workerApi.createWorker),
};

export { WindowGlobalScope };