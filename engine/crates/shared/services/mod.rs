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

mod auth;
mod camera;
mod clipboard;
mod codec;
mod device;
mod file;
mod game_log;
mod image_api;
mod interaction;
mod location;
mod navigate;
mod network;
mod payment;
mod scan_code;
mod share;
mod subpackage;
mod system_info;

pub use auth::AuthService;
pub use camera::CameraService;
pub use clipboard::ClipboardService;
pub use codec::CodecService;
pub use device::{
    AccelerometerService, AudioPlatformService, BatteryService, BluetoothService, CompassService,
    DeviceMotionService, DeviceServices, GyroscopeService, KeyboardService, RecorderService,
    ScreenService, VibrationService,
};
pub use file::FileService;
pub use game_log::GameLogService;
pub use image_api::ImageApiService;
pub use interaction::InteractionService;
pub use location::LocationService;
pub use navigate::NavigateService;
pub use network::NetworkService;
pub use payment::PaymentService;
pub use scan_code::ScanCodeService;
pub use share::ShareService;
pub use subpackage::SubpackageService;
pub use system_info::SystemInfoService;
