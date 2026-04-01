//! WebGPU API implementation for Migo.
//!
//! Maps the WebGPU JavaScript API to a native GPU backend.
//! When compiled with the `webgpu` feature, uses wgpu-core for Vulkan/GLES.
//!
//! # Architecture
//!
//! Game code creates a WebGPU context via `canvas.getContext('webgpu')`.
//! The JS API maps to Rust ops which delegate to wgpu types:
//!
//! ```text
//! JS: GPUDevice         -> Rust: wgpu::Device
//! JS: GPUBuffer         -> Rust: wgpu::Buffer
//! JS: GPURenderPipeline -> Rust: wgpu::RenderPipeline
//! JS: GPUCommandEncoder -> Rust: wgpu::CommandEncoder
//! ```

pub mod types;
pub mod adapter;
