//! GPU-accelerated Canvas2D rendering via compute shaders (GLES 3.1+).
//!
//! Divides canvas into 16x16 tiles. Two-pass architecture:
//! 1. Path coverage: compute shader determines which tiles each path covers
//! 2. Tile compositing: compute shader renders covered tiles to framebuffer
//!
//! Falls back to femtovg (existing path) on devices without compute support.

pub mod capabilities;
pub mod shaders;
pub mod tile_rasterizer;
