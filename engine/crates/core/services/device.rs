//! Device service traits re-exported from shared.
//!
//! See [`shared::services`] for the actual trait definitions.

pub use shared::services::{
    AccelerometerService, AudioPlatformService, RecorderService, BatteryService, ClipboardService,
    CameraService, CompassService, DeviceMotionService, DeviceServices, GyroscopeService,
    InteractionService, NetworkService, ScreenService, VibrationService,
};
