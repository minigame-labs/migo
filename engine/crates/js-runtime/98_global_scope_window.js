import * as lifecycle from "ext:host_v8_lifecycle/02_restart_exit.js";
import * as alert from "ext:host_v8_console/01_alert.js";
import * as url from "ext:host_v8_url/03_url.js";
import * as subpackage from 'ext:host_v8_base/04_subpackage.js';
import * as location from "ext:host_v8_web/12_location.js";
import * as performance from "ext:host_v8_web/12_performance.js";
import * as raf from "ext:host_v8_webgl/03_raf.js";
import * as fontApi from "ext:host_v8_webgl/04_font.js";
import * as canvas from "ext:host_v8_web/03_canvas.js";
import * as webgl from 'ext:host_v8_webgl/02_webgl_context.js';
import * as touch from 'ext:host_v8_touch/01_touch.js';
import * as keyboard from 'ext:host_v8_touch/02_keyboard.js';
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
import * as memory from 'ext:host_v8_device/11_memory.js';
import * as locationApi from 'ext:host_v8_device/12_location.js';
import * as scanCodeApi from 'ext:host_v8_device/13_scan_code.js';
import * as interaction from 'ext:host_v8_ui/01_interaction.js';
import * as buttonsApi from 'ext:host_v8_ui/02_buttons.js';
import * as pageManagerApi from 'ext:host_v8_ui/03_page_manager.js';
import * as envApi from 'ext:host_v8_env/00_env.js';
import * as appLifecycle from 'ext:host_v8_lifecycle/01_lifecycle.js';
import * as updateApp from 'ext:host_v8_update/01_update_app.js';
import * as updateMgr from 'ext:host_v8_update/02_update_mgr.js';
import * as bluetooth from 'ext:host_v8_system/01_bluetooth.js';
import * as authorize from 'ext:host_v8_system/02_authorize.js';
import * as windowInfo from 'ext:host_v8_system/03_window_info.js';
import * as systemSetting from 'ext:host_v8_system/04_system_settings.js';
import * as deviceInfo from 'ext:host_v8_system/05_device_info.js';
import * as benchmarkLevel from 'ext:host_v8_system/06_benchmark_level.js';
import * as appInfo from 'ext:host_v8_system/07_app_info.js';
import * as authorizeSetting from 'ext:host_v8_system/08_authorize_setting.js';
import * as gameLog from 'ext:host_v8_system/09_game_log.js';
import * as systemInfo from 'ext:host_v8_system/10_system_info.js';
import * as openDataContext from 'ext:host_v8_system/11_open_data_context.js';
import * as windowResize from 'ext:host_v8_system/12_window_resize.js';
import * as loginApi from 'ext:host_v8_system/13_login.js';
import * as storageApi from 'ext:host_v8_storage/01_storage.js';
import * as cameraApi from 'ext:host_v8_media/01_camera.js';
import * as imageApi from 'ext:host_v8_media/02_image_api.js';
import * as workerApi from 'ext:host_v8_worker/01_worker.js';
import * as settingApi from 'ext:host_v8_system/14_setting.js';
import * as navigateApi from 'ext:host_v8_system/15_navigate.js';
import * as shareApi from 'ext:host_v8_share/01_share.js';
import * as paymentApi from 'ext:host_v8_payment/01_payment.js';
import * as jssdkApi from 'ext:host_v8_system/16_jssdk.js';
import * as adApi from 'ext:host_v8_ad/01_ad.js';
import * as tcpSocket from 'ext:host_v8_network/08_tcp_socket.js';
import * as udpSocket from 'ext:host_v8_network/09_udp_socket.js';

import { core } from "ext:core/mod.js";

const WindowGlobalScope = {
    alert: core.propWritable(alert.alert),
    URL: core.propNonEnumerable(url.URL),
    location: core.propNonEnumerable(location.location),
    createCanvas: core.propNonEnumerable(canvas.createCanvas),
    requestAnimationFrame: core.propWritable(raf.requestAnimationFrame),
    cancelAnimationFrame: core.propWritable(raf.cancelAnimationFrame),
    setPreferredFramesPerSecond: core.propWritable(raf.setPreferredFramesPerSecond),

    // Subpackage
    loadSubpackage: core.propNonEnumerable(subpackage.loadSubpackage),
    preDownloadSubpackage: core.propNonEnumerable(subpackage.preDownloadSubpackage),
    _internalOnSubpackageProgress: core.propNonEnumerable(subpackage._internalOnSubpackageProgress),
    _internalOnSubpackageResult: core.propNonEnumerable(subpackage._internalOnSubpackageResult),

    WebGLRenderingContext: core.propNonEnumerable(webgl.WebGLRenderingContext),
    WebGL2RenderingContext: core.propNonEnumerable(webgl.WebGL2RenderingContext),

    performance: core.propNonEnumerable(performance.performance),
    getPerformance: core.propNonEnumerable(performance.getPerformance),

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

    // Keyboard (soft keyboard)
    showKeyboard: core.propNonEnumerable(keyboard.showKeyboard),
    hideKeyboard: core.propNonEnumerable(keyboard.hideKeyboard),
    updateKeyboard: core.propNonEnumerable(keyboard.updateKeyboard),
    onKeyboardInput: core.propNonEnumerable(keyboard.onKeyboardInput),
    offKeyboardInput: core.propNonEnumerable(keyboard.offKeyboardInput),
    onKeyboardHeightChange: core.propNonEnumerable(keyboard.onKeyboardHeightChange),
    offKeyboardHeightChange: core.propNonEnumerable(keyboard.offKeyboardHeightChange),
    onKeyboardConfirm: core.propNonEnumerable(keyboard.onKeyboardConfirm),
    offKeyboardConfirm: core.propNonEnumerable(keyboard.offKeyboardConfirm),
    onKeyboardComplete: core.propNonEnumerable(keyboard.onKeyboardComplete),
    offKeyboardComplete: core.propNonEnumerable(keyboard.offKeyboardComplete),
    _internalTriggerKeyboardInput: core.propNonEnumerable(keyboard._internalTriggerKeyboardInput),
    _internalTriggerKeyboardHeightChange: core.propNonEnumerable(keyboard._internalTriggerKeyboardHeightChange),
    _internalTriggerKeyboardConfirm: core.propNonEnumerable(keyboard._internalTriggerKeyboardConfirm),
    _internalTriggerKeyboardComplete: core.propNonEnumerable(keyboard._internalTriggerKeyboardComplete),

    // Keyboard (PC physical keys)
    onKeyDown: core.propNonEnumerable(keyboard.onKeyDown),
    offKeyDown: core.propNonEnumerable(keyboard.offKeyDown),
    onKeyUp: core.propNonEnumerable(keyboard.onKeyUp),
    offKeyUp: core.propNonEnumerable(keyboard.offKeyUp),
    _internalTriggerKeyDown: core.propNonEnumerable(keyboard._internalTriggerKeyDown),
    _internalTriggerKeyUp: core.propNonEnumerable(keyboard._internalTriggerKeyUp),

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
    setEnableDebug: core.propNonEnumerable(screen.setEnableDebug),
    onUserCaptureScreen: core.propNonEnumerable(screen.onUserCaptureScreen),
    offUserCaptureScreen: core.propNonEnumerable(screen.offUserCaptureScreen),
    _internalTriggerUserCaptureScreen: core.propNonEnumerable(screen._internalTriggerUserCaptureScreen),

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

    // Memory
    onMemoryWarning: core.propNonEnumerable(memory.onMemoryWarning),
    offMemoryWarning: core.propNonEnumerable(memory.offMemoryWarning),
    _internalTriggerMemoryWarning: core.propNonEnumerable(memory._internalTriggerMemoryWarning),

    // Location
    getLocation: core.propNonEnumerable(locationApi.getLocation),
    getFuzzyLocation: core.propNonEnumerable(locationApi.getFuzzyLocation),
    _internalOnLocationResult: core.propNonEnumerable(locationApi._internalOnLocationResult),
    _internalOnFuzzyLocationResult: core.propNonEnumerable(locationApi._internalOnFuzzyLocationResult),

    // Scan Code
    scanCode: core.propNonEnumerable(scanCodeApi.scanCode),
    _internalOnScanCodeResult: core.propNonEnumerable(scanCodeApi._internalOnScanCodeResult),

    // Storage
    setStorage: core.propNonEnumerable(storageApi.setStorage),
    setStorageSync: core.propNonEnumerable(storageApi.setStorageSync),
    getStorage: core.propNonEnumerable(storageApi.getStorage),
    getStorageSync: core.propNonEnumerable(storageApi.getStorageSync),
    removeStorage: core.propNonEnumerable(storageApi.removeStorage),
    removeStorageSync: core.propNonEnumerable(storageApi.removeStorageSync),
    clearStorage: core.propNonEnumerable(storageApi.clearStorage),
    clearStorageSync: core.propNonEnumerable(storageApi.clearStorageSync),
    getStorageInfo: core.propNonEnumerable(storageApi.getStorageInfo),
    getStorageInfoSync: core.propNonEnumerable(storageApi.getStorageInfoSync),
    createBufferURL: core.propNonEnumerable(storageApi.createBufferURL),
    revokeBufferURL: core.propNonEnumerable(storageApi.revokeBufferURL),

    // Camera
    createCamera: core.propNonEnumerable(cameraApi.createCamera),
    _internalOnCameraEvent: core.propNonEnumerable(cameraApi._internalOnCameraEvent),
    _internalOnCameraFrameData: core.propNonEnumerable(cameraApi._internalOnCameraFrameData),

    // Image API
    saveImageToPhotosAlbum: core.propNonEnumerable(imageApi.saveImageToPhotosAlbum),
    previewMedia: core.propNonEnumerable(imageApi.previewMedia),
    previewImage: core.propNonEnumerable(imageApi.previewImage),
    compressImage: core.propNonEnumerable(imageApi.compressImage),
    _internalOnCompressImageResult: core.propNonEnumerable(imageApi._internalOnCompressImageResult),
    chooseMessageFile: core.propNonEnumerable(imageApi.chooseMessageFile),
    chooseImage: core.propNonEnumerable(imageApi.chooseImage),
    _internalOnChooseMessageFileResult: core.propNonEnumerable(imageApi._internalOnChooseMessageFileResult),
    _internalOnChooseImageResult: core.propNonEnumerable(imageApi._internalOnChooseImageResult),

    // Worker
    createWorker: core.propNonEnumerable(workerApi.createWorker),

    // UI Interaction
    showToast: core.propNonEnumerable(interaction.showToast),
    hideToast: core.propNonEnumerable(interaction.hideToast),
    showModal: core.propNonEnumerable(interaction.showModal),
    _internalOnModalResult: core.propNonEnumerable(interaction._internalOnModalResult),
    showLoading: core.propNonEnumerable(interaction.showLoading),
    hideLoading: core.propNonEnumerable(interaction.hideLoading),
    showActionSheet: core.propNonEnumerable(interaction.showActionSheet),
    _internalOnActionSheetResult: core.propNonEnumerable(interaction._internalOnActionSheetResult),

    // UI Buttons
    createUserInfoButton: core.propNonEnumerable(buttonsApi.createUserInfoButton),
    createGameClubButton: core.propNonEnumerable(buttonsApi.createGameClubButton),
    getMenuButtonBoundingClientRect: core.propNonEnumerable(buttonsApi.getMenuButtonBoundingClientRect),

    // Page Manager
    createPageManager: core.propNonEnumerable(pageManagerApi.createPageManager),

    // Env
    env: core.propNonEnumerable(envApi.env),

    // App Lifecycle (show/hide)
    onShow: core.propNonEnumerable(appLifecycle.onShow),
    onHide: core.propNonEnumerable(appLifecycle.onHide),
    offShow: core.propNonEnumerable(appLifecycle.offShow),
    offHide: core.propNonEnumerable(appLifecycle.offHide),
    getLaunchOptionsSync: core.propNonEnumerable(appLifecycle.getLaunchOptionsSync),
    getEnterOptionsSync: core.propNonEnumerable(appLifecycle.getEnterOptionsSync),
    _internalTriggerOnShow: core.propNonEnumerable(appLifecycle._internalTriggerOnShow),
    _internalTriggerOnHide: core.propNonEnumerable(appLifecycle._internalTriggerOnHide),

    // App Lifecycle (restart/exit)
    restartMiniProgram: core.propNonEnumerable(lifecycle.restartMiniProgram),
    exitMiniProgram: core.propNonEnumerable(lifecycle.exitMiniProgram),

    // Update
    updateApp: core.propNonEnumerable(updateApp.updateApp),
    getUpdateManager: core.propNonEnumerable(updateMgr.getUpdateManager),

    // Bluetooth
    openSystemBluetoothSetting: core.propNonEnumerable(bluetooth.openSystemBluetoothSetting),
    _internalOnOpenBluetoothSettingFinished: core.propNonEnumerable(bluetooth._internalOnOpenBluetoothSettingFinished),
    openBluetoothAdapter: core.propNonEnumerable(bluetooth.openBluetoothAdapter),
    closeBluetoothAdapter: core.propNonEnumerable(bluetooth.closeBluetoothAdapter),
    getBluetoothAdapterState: core.propNonEnumerable(bluetooth.getBluetoothAdapterState),
    startBluetoothDevicesDiscovery: core.propNonEnumerable(bluetooth.startBluetoothDevicesDiscovery),
    stopBluetoothDevicesDiscovery: core.propNonEnumerable(bluetooth.stopBluetoothDevicesDiscovery),
    getBluetoothDevices: core.propNonEnumerable(bluetooth.getBluetoothDevices),
    getConnectedBluetoothDevices: core.propNonEnumerable(bluetooth.getConnectedBluetoothDevices),
    makeBluetoothPair: core.propNonEnumerable(bluetooth.makeBluetoothPair),
    isBluetoothDevicePaired: core.propNonEnumerable(bluetooth.isBluetoothDevicePaired),
    onBluetoothAdapterStateChange: core.propNonEnumerable(bluetooth.onBluetoothAdapterStateChange),
    offBluetoothAdapterStateChange: core.propNonEnumerable(bluetooth.offBluetoothAdapterStateChange),
    onBluetoothDeviceFound: core.propNonEnumerable(bluetooth.onBluetoothDeviceFound),
    offBluetoothDeviceFound: core.propNonEnumerable(bluetooth.offBluetoothDeviceFound),
    _internalTriggerBluetoothAdapterStateChange: core.propNonEnumerable(bluetooth._internalTriggerBluetoothAdapterStateChange),
    _internalTriggerBluetoothDeviceFound: core.propNonEnumerable(bluetooth._internalTriggerBluetoothDeviceFound),
    // Beacon
    startBeaconDiscovery: core.propNonEnumerable(bluetooth.startBeaconDiscovery),
    stopBeaconDiscovery: core.propNonEnumerable(bluetooth.stopBeaconDiscovery),
    getBeacons: core.propNonEnumerable(bluetooth.getBeacons),
    onBeaconUpdate: core.propNonEnumerable(bluetooth.onBeaconUpdate),
    offBeaconUpdate: core.propNonEnumerable(bluetooth.offBeaconUpdate),
    onBeaconServiceChange: core.propNonEnumerable(bluetooth.onBeaconServiceChange),
    offBeaconServiceChange: core.propNonEnumerable(bluetooth.offBeaconServiceChange),
    _internalTriggerBeaconUpdate: core.propNonEnumerable(bluetooth._internalTriggerBeaconUpdate),
    _internalTriggerBeaconServiceChange: core.propNonEnumerable(bluetooth._internalTriggerBeaconServiceChange),

    // Authorize
    openAppAuthorizeSetting: core.propNonEnumerable(authorize.openAppAuthorizeSetting),
    _internalOnOpenAppAuthorizeSettingFinished: core.propNonEnumerable(authorize._internalOnOpenAppAuthorizeSettingFinished),

    // Setting / Authorize (getSetting, authorize, openSetting)
    getSetting: core.propNonEnumerable(settingApi.getSetting),
    authorize: core.propNonEnumerable(settingApi.authorize),
    openSetting: core.propNonEnumerable(settingApi.openSetting),
    _internalOnOpenSettingResult: core.propNonEnumerable(settingApi._internalOnOpenSettingResult),
    _internalUpdateAuthSetting: core.propNonEnumerable(settingApi._internalUpdateAuthSetting),

    // Navigate / Customer Service
    navigateToMiniProgram: core.propNonEnumerable(navigateApi.navigateToMiniProgram),
    _internalOnNavigateToMiniProgramResult: core.propNonEnumerable(navigateApi._internalOnNavigateToMiniProgramResult),
    openCustomerServiceConversation: core.propNonEnumerable(navigateApi.openCustomerServiceConversation),

    // Login
    login: core.propNonEnumerable(loginApi.login),
    checkSession: core.propNonEnumerable(loginApi.checkSession),
    getUserInfo: core.propNonEnumerable(loginApi.getUserInfo),
    getPhoneNumber: core.propNonEnumerable(loginApi.getPhoneNumber),
    _internalOnLoginResult: core.propNonEnumerable(loginApi._internalOnLoginResult),
    _internalOnCheckSessionResult: core.propNonEnumerable(loginApi._internalOnCheckSessionResult),
    _internalOnGetUserInfoResult: core.propNonEnumerable(loginApi._internalOnGetUserInfoResult),
    _internalOnGetPhoneNumberResult: core.propNonEnumerable(loginApi._internalOnGetPhoneNumberResult),

    // Window Info
    getWindowInfo: core.propNonEnumerable(windowInfo.getWindowInfo),

    // System Setting
    getSystemSetting: core.propNonEnumerable(systemSetting.getSystemSetting),

    // Device Info
    getDeviceInfo: core.propNonEnumerable(deviceInfo.getDeviceInfo),

    // Benchmark Level
    getDeviceBenchmarkInfo: core.propNonEnumerable(benchmarkLevel.getDeviceBenchmarkInfo),

    // App Info
    getAppBaseInfo: core.propNonEnumerable(appInfo.getAppBaseInfo),

    // Authorize Setting
    getAppAuthorizeSetting: core.propNonEnumerable(authorizeSetting.getAppAuthorizeSetting),

    // System Info (legacy combined API)
    getSystemInfo: core.propNonEnumerable(systemInfo.getSystemInfo),
    getSystemInfoSync: core.propNonEnumerable(systemInfo.getSystemInfoSync),
    getSystemInfoAsync: core.propNonEnumerable(systemInfo.getSystemInfoAsync),

    // Open Data Context
    onMessage: core.propNonEnumerable(openDataContext.onMessage),
    offMessage: core.propNonEnumerable(openDataContext.offMessage),
    getOpenDataContext: core.propNonEnumerable(openDataContext.getOpenDataContext),
    getSharedCanvas: core.propNonEnumerable(openDataContext.getSharedCanvas),

    // Window Resize
    onWindowResize: core.propNonEnumerable(windowResize.onWindowResize),
    offWindowResize: core.propNonEnumerable(windowResize.offWindowResize),
    _internalTriggerWindowResize: core.propNonEnumerable(windowResize._internalTriggerWindowResize),

    // Game Log
    getGameLogManager: core.propNonEnumerable(gameLog.getGameLogManager),

    // Share
    showShareMenu: core.propNonEnumerable(shareApi.showShareMenu),
    updateShareMenu: core.propNonEnumerable(shareApi.updateShareMenu),
    onShareAppMessage: core.propNonEnumerable(shareApi.onShareAppMessage),
    offShareAppMessage: core.propNonEnumerable(shareApi.offShareAppMessage),
    shareAppMessage: core.propNonEnumerable(shareApi.shareAppMessage),
    _internalOnShareAppMessageResult: core.propNonEnumerable(shareApi._internalOnShareAppMessageResult),
    _internalTriggerShareAppMessage: core.propNonEnumerable(shareApi._internalTriggerShareAppMessage),

    // Ad
    createBannerAd: core.propNonEnumerable(adApi.createBannerAd),
    createCustomAd: core.propNonEnumerable(adApi.createCustomAd),
    createGridAd: core.propNonEnumerable(adApi.createGridAd),
    createInterstitialAd: core.propNonEnumerable(adApi.createInterstitialAd),
    createRewardedVideoAd: core.propNonEnumerable(adApi.createRewardedVideoAd),
    getDirectAdStatusSync: core.propNonEnumerable(adApi.getDirectAdStatusSync),
    getShowSplashAdStatus: core.propNonEnumerable(adApi.getShowSplashAdStatus),
    createGameBanner: core.propNonEnumerable(adApi.createGameBanner),
    createGameIcon: core.propNonEnumerable(adApi.createGameIcon),
    createGamePortal: core.propNonEnumerable(adApi.createGamePortal),

    // Payment
    checkIsSupportMidasPayment: core.propNonEnumerable(paymentApi.checkIsSupportMidasPayment),
    requestMidasPayment: core.propNonEnumerable(paymentApi.requestMidasPayment),
    requestMidasPaymentGameItem: core.propNonEnumerable(paymentApi.requestMidasPaymentGameItem),
    _internalOnMidasPaymentResult: core.propNonEnumerable(paymentApi._internalOnMidasPaymentResult),
    _internalOnMidasPaymentGameItemResult: core.propNonEnumerable(paymentApi._internalOnMidasPaymentGameItemResult),

    // JSSDK Lifecycle (config / ready / error)
    config: core.propNonEnumerable(jssdkApi.config),
    ready: core.propNonEnumerable(jssdkApi.ready),
    error: core.propNonEnumerable(jssdkApi.error),
    _internalTriggerJssdkError: core.propNonEnumerable(jssdkApi._internalTriggerJssdkError),

    // TCP Socket
    createTCPSocket: core.propNonEnumerable(tcpSocket.createTCPSocket),

    // UDP Socket
    createUDPSocket: core.propNonEnumerable(udpSocket.createUDPSocket),
};

export { WindowGlobalScope };
