//! Device service traits re-exported from shared.
//!
//! See [`shared::services`] for the actual trait definitions.

pub use shared::services::{
    AccelerometerService, AudioPlatformService, AuthService, BatteryService, BluetoothService,
    CameraService, ClipboardService, CodecService, CommerceServices, CompassService,
    ConnectivityServices, DeviceMotionService, DeviceServices, FileService, GameLogService,
    GyroscopeService, ImageApiService, InteractionService, KeyboardService, LocationService,
    MediaServices, NavigateService, NetworkService, PaymentService, RecorderService,
    ScanCodeService, ScreenService, SensorServices, ServiceError, ServiceErrorCode, ShareService,
    SubpackageService, SystemInfoService, SystemUtilServices, VibrationService, VideoService,
};
