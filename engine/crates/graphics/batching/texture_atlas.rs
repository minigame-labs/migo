//! Texture Atlas for reducing texture switches
//!
//! Packs multiple small images into a single large texture to minimize
//! GPU state changes during rendering.

use std::collections::HashMap;

use glow::HasContext;
use shared::error::{EngineResult, ErrorCode, EngineError};
use shared::protocol::io_cmd::NormalizedImage;

/// Region within an atlas
#[derive(Debug, Clone, Copy)]
pub struct AtlasRegion {
    /// X position in atlas (pixels)
    pub x: u32,
    /// Y position in atlas (pixels)
    pub y: u32,
    /// Width (pixels)
    pub width: u32,
    /// Height (pixels)
    pub height: u32,
    /// UV coordinates (normalized)
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
}

/// A node in the binary tree packing algorithm
#[derive(Debug)]
struct PackNode {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    /// Index of image stored here (if any)
    image_id: Option<u32>,
    /// Child nodes (right, bottom)
    children: Option<Box<(PackNode, PackNode)>>,
}

impl PackNode {
    fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
            image_id: None,
            children: None,
        }
    }
    
    /// Try to insert an image into this node or its children
    fn insert(&mut self, image_id: u32, w: u32, h: u32, padding: u32) -> Option<(u32, u32)> {
        let pw = w + padding * 2;
        let ph = h + padding * 2;
        
        // If we have children, try them first
        if let Some(ref mut children) = self.children {
            if let Some(pos) = children.0.insert(image_id, w, h, padding) {
                return Some(pos);
            }
            return children.1.insert(image_id, w, h, padding);
        }
        
        // If already occupied, can't insert here
        if self.image_id.is_some() {
            return None;
        }
        
        // Check if image fits
        if pw > self.width || ph > self.height {
            return None;
        }
        
        // Perfect fit?
        if pw == self.width && ph == self.height {
            self.image_id = Some(image_id);
            return Some((self.x + padding, self.y + padding));
        }
        
        // Split the node
        let dw = self.width - pw;
        let dh = self.height - ph;
        
        let (right, bottom) = if dw > dh {
            // Split vertically
            (
                PackNode::new(self.x + pw, self.y, dw, self.height),
                PackNode::new(self.x, self.y + ph, pw, dh),
            )
        } else {
            // Split horizontally
            (
                PackNode::new(self.x + pw, self.y, dw, ph),
                PackNode::new(self.x, self.y + ph, self.width, dh),
            )
        };
        
        self.children = Some(Box::new((right, bottom)));
        
        // Resize this node to fit the image exactly
        self.width = pw;
        self.height = ph;
        self.image_id = Some(image_id);
        
        Some((self.x + padding, self.y + padding))
    }
}

/// Texture atlas for packing multiple images
pub struct TextureAtlas {
    /// OpenGL texture handle
    texture: glow::NativeTexture,
    /// Atlas dimensions
    width: u32,
    height: u32,
    /// Packing tree root
    root: PackNode,
    /// Stored regions by image ID
    regions: HashMap<u32, AtlasRegion>,
    /// Padding between images (in pixels)
    padding: u32,
    /// Whether atlas needs GPU upload
    dirty: bool,
    /// CPU-side pixel data
    pixels: Vec<u8>,
}

impl TextureAtlas {
    /// Default atlas size (2048x2048)
    pub const DEFAULT_SIZE: u32 = 2048;
    
    /// Maximum atlas size
    pub const MAX_SIZE: u32 = 4096;
    
    /// Default padding between images
    pub const DEFAULT_PADDING: u32 = 1;
    
    /// Create a new texture atlas
    pub fn new(gl: &glow::Context, width: u32, height: u32) -> EngineResult<Self> {
        let width = width.min(Self::MAX_SIZE);
        let height = height.min(Self::MAX_SIZE);
        
        // Create GL texture
        let texture = unsafe {
            gl.create_texture().map_err(|e| {
                EngineError::new(ErrorCode::RenderBackendError)
                    .with_detail(format!("Failed to create atlas texture: {e}"))
            })?
        };
        
        // Allocate texture storage
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
            
            // Allocate empty texture
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA as i32,
                width as i32,
                height as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            gl.bind_texture(glow::TEXTURE_2D, None);
        }
        
        Ok(Self {
            texture,
            width,
            height,
            root: PackNode::new(0, 0, width, height),
            regions: HashMap::with_capacity(64),
            padding: Self::DEFAULT_PADDING,
            dirty: false,
            pixels: vec![0u8; (width * height * 4) as usize],
        })
    }
    
    /// Get the GL texture handle
    pub fn texture(&self) -> glow::NativeTexture {
        self.texture
    }
    
    /// Get atlas dimensions
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    
    /// Set padding between images
    pub fn set_padding(&mut self, padding: u32) {
        self.padding = padding;
    }
    
    /// Add an image to the atlas
    pub fn add_image(&mut self, image_id: u32, image: &NormalizedImage) -> EngineResult<AtlasRegion> {
        // Check if already added
        if let Some(region) = self.regions.get(&image_id) {
            return Ok(*region);
        }
        
        // Try to pack
        let pos = self.root.insert(image_id, image.width, image.height, self.padding)
            .ok_or_else(|| {
                EngineError::new(ErrorCode::RenderBackendError)
                    .with_detail("Atlas is full, cannot add image")
            })?;
        
        // Copy pixels to atlas
        let (x, y) = pos;
        for row in 0..image.height {
            let src_offset = (row * image.width * 4) as usize;
            let dst_offset = ((y + row) * self.width + x) as usize * 4;
            let row_size = (image.width * 4) as usize;
            
            self.pixels[dst_offset..dst_offset + row_size]
                .copy_from_slice(&image.rgba[src_offset..src_offset + row_size]);
        }
        
        // Calculate UV coordinates
        let u0 = x as f32 / self.width as f32;
        let v0 = y as f32 / self.height as f32;
        let u1 = (x + image.width) as f32 / self.width as f32;
        let v1 = (y + image.height) as f32 / self.height as f32;
        
        let region = AtlasRegion {
            x, y,
            width: image.width,
            height: image.height,
            u0, v0, u1, v1,
        };
        
        self.regions.insert(image_id, region);
        self.dirty = true;
        
        Ok(region)
    }
    
    /// Get region for an image
    pub fn get_region(&self, image_id: u32) -> Option<AtlasRegion> {
        self.regions.get(&image_id).copied()
    }
    
    /// Check if atlas contains an image
    pub fn contains(&self, image_id: u32) -> bool {
        self.regions.contains_key(&image_id)
    }
    
    /// Upload dirty regions to GPU
    pub fn upload(&mut self, gl: &glow::Context) {
        if !self.dirty {
            return;
        }
        
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
            gl.tex_sub_image_2d(
                glow::TEXTURE_2D,
                0,
                0,
                0,
                self.width as i32,
                self.height as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&self.pixels)),
            );
            gl.bind_texture(glow::TEXTURE_2D, None);
        }
        
        self.dirty = false;
    }
    
    /// Get number of images in atlas
    pub fn image_count(&self) -> usize {
        self.regions.len()
    }
    
    /// Calculate fill percentage
    pub fn fill_percentage(&self) -> f32 {
        let used: u32 = self.regions.values()
            .map(|r| r.width * r.height)
            .sum();
        let total = self.width * self.height;
        used as f32 / total as f32 * 100.0
    }
    
    /// Destroy the atlas
    pub fn destroy(&mut self, gl: &glow::Context) {
        unsafe {
            gl.delete_texture(self.texture);
        }
        self.regions.clear();
        self.pixels.clear();
    }
}

/// Manages multiple texture atlases
pub struct AtlasManager {
    /// Atlases by category
    atlases: Vec<TextureAtlas>,
    /// Image ID to atlas index mapping
    image_atlas_map: HashMap<u32, usize>,
    /// Default atlas size
    atlas_size: u32,
    /// Maximum images that should use atlas
    max_image_size: u32,
}

impl AtlasManager {
    /// Maximum image size to consider for atlas packing
    const MAX_ATLAS_IMAGE_SIZE: u32 = 512;
    
    /// Create a new atlas manager
    pub fn new(atlas_size: u32) -> Self {
        Self {
            atlases: Vec::with_capacity(4),
            image_atlas_map: HashMap::with_capacity(64),
            atlas_size,
            max_image_size: Self::MAX_ATLAS_IMAGE_SIZE,
        }
    }
    
    /// Check if an image should be atlased
    pub fn should_atlas(&self, width: u32, height: u32) -> bool {
        width <= self.max_image_size && height <= self.max_image_size
    }
    
    /// Add an image to the manager
    pub fn add_image(
        &mut self,
        gl: &glow::Context,
        image_id: u32,
        image: &NormalizedImage,
    ) -> EngineResult<Option<(glow::NativeTexture, AtlasRegion)>> {
        // Skip if too large
        if !self.should_atlas(image.width, image.height) {
            return Ok(None);
        }
        
        // Try existing atlases
        for (idx, atlas) in self.atlases.iter_mut().enumerate() {
            if let Ok(region) = atlas.add_image(image_id, image) {
                self.image_atlas_map.insert(image_id, idx);
                return Ok(Some((atlas.texture(), region)));
            }
        }
        
        // Create new atlas
        let mut atlas = TextureAtlas::new(gl, self.atlas_size, self.atlas_size)?;
        let region = atlas.add_image(image_id, image)?;
        let texture = atlas.texture();
        
        self.image_atlas_map.insert(image_id, self.atlases.len());
        self.atlases.push(atlas);
        
        Ok(Some((texture, region)))
    }
    
    /// Get image info
    pub fn get_image(&self, image_id: u32) -> Option<(glow::NativeTexture, AtlasRegion)> {
        let idx = *self.image_atlas_map.get(&image_id)?;
        let atlas = self.atlases.get(idx)?;
        let region = atlas.get_region(image_id)?;
        Some((atlas.texture(), region))
    }
    
    /// Upload all dirty atlases
    pub fn upload_all(&mut self, gl: &glow::Context) {
        for atlas in &mut self.atlases {
            atlas.upload(gl);
        }
    }
    
    /// Get statistics
    pub fn stats(&self) -> (usize, usize, f32) {
        let atlas_count = self.atlases.len();
        let image_count = self.image_atlas_map.len();
        let avg_fill = if atlas_count > 0 {
            self.atlases.iter().map(|a| a.fill_percentage()).sum::<f32>() / atlas_count as f32
        } else {
            0.0
        };
        (atlas_count, image_count, avg_fill)
    }
    
    /// Destroy all atlases
    pub fn destroy(&mut self, gl: &glow::Context) {
        for atlas in &mut self.atlases {
            atlas.destroy(gl);
        }
        self.atlases.clear();
        self.image_atlas_map.clear();
    }
}
