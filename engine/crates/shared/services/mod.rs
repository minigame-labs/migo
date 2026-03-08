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

mod camera;
mod clipboard;
mod codec;
mod device;
mod file;
mod interaction;
mod network;
mod system_info;

pub use camera::CameraService;
pub use clipboard::ClipboardService;
pub use codec::CodecService;
pub use device::{
    AccelerometerService, AudioPlatformService, BatteryService, BluetoothService, CompassService,
    DeviceMotionService, DeviceServices, GyroscopeService, KeyboardService, RecorderService,
    ScreenService, VibrationService,
};
pub use file::FileService;
pub use interaction::InteractionService;
pub use network::NetworkService;
pub use system_info::SystemInfoService;
