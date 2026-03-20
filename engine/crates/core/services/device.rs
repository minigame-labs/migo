//! Device service traits re-exported from shared.
//!
//! See [`shared::services`] for the actual trait definitions.

pub use shared::services::{
    AccelerometerService, AudioPlatformService, AuthService, BatteryService, BluetoothService,
    CameraService, ClipboardService, CodecService, CompassService, DeviceMotionService,
    DeviceServices, FileService, GameLogService, GyroscopeService, ImageApiService,
    InteractionService, KeyboardService, LocationService, NavigateService, NetworkService,
    PaymentService, RecorderService, ScanCodeService, ScreenService, ShareService,
    SubpackageService, SystemInfoService, VibrationService,
};
