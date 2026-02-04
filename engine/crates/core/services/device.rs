//! Device service traits re-exported from shared.
//!
//! See [`shared::services`] for the actual trait definitions.

pub use shared::services::{
    AccelerometerService, BatteryService, ClipboardService, CompassService, DeviceMotionService,
    DeviceServices, GyroscopeService, NetworkService, ScreenService, VibrationService,
};
