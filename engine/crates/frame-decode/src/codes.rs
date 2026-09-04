//! WebGL error codes.
//!
//! The values are the GL enum values, not an internal numbering: they are what
//! `gl.getError()` returns to content, so a different number here would be a
//! different error as far as the game is concerned.

pub const NO_ERROR: u32 = 0;
pub const INVALID_ENUM: u32 = 0x0500;
pub const INVALID_VALUE: u32 = 0x0501;
pub const INVALID_OPERATION: u32 = 0x0502;
pub const OUT_OF_MEMORY: u32 = 0x0505;
pub const INVALID_FRAMEBUFFER_OPERATION: u32 = 0x0506;
pub const CONTEXT_LOST_WEBGL: u32 = 0x9242;
