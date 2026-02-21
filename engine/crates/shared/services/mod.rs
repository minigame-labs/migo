//! Platform service traits for cross-platform device abstractions.
//!
//! These traits define the interface for device capabilities. Each platform
//! implements these traits, and ops in js-runtime call them through HostOpState.
//!
//! # Error Convention
//!
//! Methods return `Err("apiName:fail reason")`
//! - `Err("vibrateShort:fail not supported")` - Feature not supported
//! - `Err("vibrateShort:fail system error")` - Runtime error

mod clipboard;
mod device;
mod network;

pub use clipboard::ClipboardService;
pub use device::{
    AccelerometerService, AudioPlatformService, BatteryService, CompassService,
    DeviceMotionService, DeviceServices, GyroscopeService, RecorderService, ScreenService,
    VibrationService,
};
pub use network::NetworkService;
