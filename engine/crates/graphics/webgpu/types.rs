//! WebGPU type definitions matching the WebGPU spec.
//! These are the Rust-side representations of WebGPU JavaScript objects.

/// GPU adapter info returned by navigator.gpu.requestAdapter().
#[derive(Debug, Clone)]
pub struct GpuAdapterInfo {
    pub vendor: String,
    pub architecture: String,
    pub device: String,
    pub description: String,
}

/// Supported texture formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum GpuTextureFormat {
    Rgba8Unorm = 0,
    Rgba8UnormSrgb = 1,
    Bgra8Unorm = 2,
    Bgra8UnormSrgb = 3,
    Rgb10a2Unorm = 4,
    Rgba16Float = 5,
}

/// GPU buffer usage flags (matching WebGPU spec).
#[derive(Debug, Clone, Copy)]
pub struct GpuBufferUsage(pub u32);

impl GpuBufferUsage {
    pub const MAP_READ: u32 = 0x0001;
    pub const MAP_WRITE: u32 = 0x0002;
    pub const COPY_SRC: u32 = 0x0004;
    pub const COPY_DST: u32 = 0x0008;
    pub const INDEX: u32 = 0x0010;
    pub const VERTEX: u32 = 0x0020;
    pub const UNIFORM: u32 = 0x0040;
    pub const STORAGE: u32 = 0x0080;
    pub const INDIRECT: u32 = 0x0100;
    pub const QUERY_RESOLVE: u32 = 0x0200;
}

/// Preferred canvas format for WebGPU output.
#[derive(Debug, Clone, Copy)]
pub enum PreferredCanvasFormat {
    Rgba8Unorm,
    Bgra8Unorm,
}

/// WebGPU feature names (subset most commonly used).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GpuFeatureName {
    DepthClipControl,
    Depth32FloatStencil8,
    TextureCompressionBc,
    TextureCompressionEtc2,
    TextureCompressionAstc,
    TimestampQuery,
    IndirectFirstInstance,
    ShaderF16,
    Float32Filterable,
}

/// GPU limits matching WebGPU spec defaults.
#[derive(Debug, Clone)]
pub struct GpuLimits {
    pub max_texture_dimension_1d: u32,
    pub max_texture_dimension_2d: u32,
    pub max_texture_dimension_3d: u32,
    pub max_texture_array_layers: u32,
    pub max_bind_groups: u32,
    pub max_bindings_per_bind_group: u32,
    pub max_buffer_size: u64,
    pub max_vertex_buffers: u32,
    pub max_vertex_attributes: u32,
    pub max_vertex_buffer_array_stride: u32,
    pub max_storage_buffer_binding_size: u32,
    pub max_compute_workgroup_size_x: u32,
    pub max_compute_workgroup_size_y: u32,
    pub max_compute_workgroup_size_z: u32,
    pub max_compute_workgroups_per_dimension: u32,
}

impl Default for GpuLimits {
    fn default() -> Self {
        Self {
            max_texture_dimension_1d: 8192,
            max_texture_dimension_2d: 8192,
            max_texture_dimension_3d: 2048,
            max_texture_array_layers: 256,
            max_bind_groups: 4,
            max_bindings_per_bind_group: 1000,
            max_buffer_size: 268435456, // 256 MB
            max_vertex_buffers: 8,
            max_vertex_attributes: 16,
            max_vertex_buffer_array_stride: 2048,
            max_storage_buffer_binding_size: 134217728, // 128 MB
            max_compute_workgroup_size_x: 256,
            max_compute_workgroup_size_y: 256,
            max_compute_workgroup_size_z: 64,
            max_compute_workgroups_per_dimension: 65535,
        }
    }
}
