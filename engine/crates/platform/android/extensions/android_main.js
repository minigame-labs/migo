import { core, primordials } from "ext:core/mod.js";

import * as env from "ext:host_v8_env/00_env.js"
import * as bluetooth from "ext:host_v8_system/01_bluetooth.js"
import * as authorize from "ext:host_v8_system/02_authorize.js"
import * as windowInfo from "ext:host_v8_system/03_window_info.js"
import * as systemSetting from "ext:host_v8_system/04_system_settings.js"
import * as deviceInfo from "ext:host_v8_system/05_device_info.js"
import * as benchmarkLevel from "ext:host_v8_system/06_benchmark_level.js"
import * as appInfo from "ext:host_v8_system/07_app_info.js"
import * as authorizeSetting from "ext:host_v8_system/08_authorize_setting.js"
import * as updateApp from "ext:host_v8_update/01_update_app.js"
import * as updateMgr from "ext:host_v8_update/02_update_mgr.js"
import * as lifeCycle from "ext:host_v8_lifecycle/01_lifecycle.js"
import * as fileManager from "ext:host_v8_file_android/01_file_manager.js"
import * as interaction from "ext:host_v8_ui/01_interaction.js"
import * as deviceSensor from "ext:host_v8_device_android/01_device_sensor.js"
import * as battery from "ext:host_v8_device_android/02_battery.js"
import * as vibrate from "ext:host_v8_device_android/03_vibrate.js"

const { ObjectDefineProperties } = primordials;
const properties = {
    // Env
    env: core.propNonEnumerable(env.env),
    // Bluetooth
    openSystemBluetoothSetting: core.propNonEnumerable(bluetooth.openSystemBluetoothSetting),
    _internalOnOpenBluetoothSettingFinished: core.propNonEnumerable(bluetooth._internalOnOpenBluetoothSettingFinished),
    // Authorize
    openAppAuthorizeSetting: core.propNonEnumerable(authorize.openAppAuthorizeSetting),
    _internalOnOpenAppAuthorizeSettingFinished: core.propNonEnumerable(authorize._internalOnOpenAppAuthorizeSettingFinished),
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
    // Update
    updateApp: core.propNonEnumerable(updateApp.updateApp),
    getUpdateManager: core.propNonEnumerable(updateMgr.getUpdateManager),
    // LifeCycle
    onShow: core.propNonEnumerable(lifeCycle.onShow),
    onHide: core.propNonEnumerable(lifeCycle.onHide),
    offShow: core.propNonEnumerable(lifeCycle.offShow),
    offHide: core.propNonEnumerable(lifeCycle.offHide),
    getLaunchOptionsSync: core.propNonEnumerable(lifeCycle.getLaunchOptionsSync),
    getEnterOptionsSync: core.propNonEnumerable(lifeCycle.getEnterOptionsSync),
    _internalTriggerOnShow: core.propNonEnumerable(lifeCycle._internalTriggerOnShow),
    _internalTriggerOnHide: core.propNonEnumerable(lifeCycle._internalTriggerOnHide),
    // File
    getFileSystemManager: core.propNonEnumerable(fileManager.getFileSystemManager),
    // UI Interaction
    showToast: core.propNonEnumerable(interaction.showToast),
    hideToast: core.propNonEnumerable(interaction.hideToast),
    showModal: core.propNonEnumerable(interaction.showModal),
    _internalOnModalResult: core.propNonEnumerable(interaction._internalOnModalResult),
    showLoading: core.propNonEnumerable(interaction.showLoading),
    hideLoading: core.propNonEnumerable(interaction.hideLoading),
    showActionSheet: core.propNonEnumerable(interaction.showActionSheet),
    _internalOnActionSheetResult: core.propNonEnumerable(interaction._internalOnActionSheetResult),
    // Device Sensor
    startDeviceMotionListening: core.propNonEnumerable(deviceSensor.startDeviceMotionListening),
    stopDeviceMotionListening: core.propNonEnumerable(deviceSensor.stopDeviceMotionListening),
    startGyroscope: core.propNonEnumerable(deviceSensor.startGyroscope),
    stopGyroscope: core.propNonEnumerable(deviceSensor.stopGyroscope),
    // Battery
    getBatteryInfo: core.propNonEnumerable(battery.getBatteryInfo),
    getBatteryInfoSync: core.propNonEnumerable(battery.getBatteryInfoSync),
    // Vibration
    vibrateShort: core.propNonEnumerable(vibrate.vibrateShort),
    vibrateLong: core.propNonEnumerable(vibrate.vibrateLong),
};

ObjectDefineProperties(globalThis, properties);