//! Device service traits re-exported from shared.
//!
//! See [`shared::services`] for the actual trait definitions.

pub use shared::services::{
    AccelerometerService, AudioPlatformService, BatteryService, CameraService, ClipboardService,
    CodecService, CompassService, DeviceMotionService, DeviceServices, FileService,
    GyroscopeService, InteractionService, NetworkService, RecorderService, ScreenService,
    SystemInfoService, VibrationService,
};
