//! Rectangle types for dirty region tracking

/// A rectangle in screen coordinates
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DirtyRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl DirtyRect {
    /// Create a new rectangle
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self { x, y, width, height }
    }
    
    /// Create a rectangle from bounds
    pub fn from_bounds(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        }
    }
    
    /// Create an empty rectangle
    pub fn empty() -> Self {
        Self::new(0.0, 0.0, 0.0, 0.0)
    }
    
    /// Check if rectangle is empty
    pub fn is_empty(&self) -> bool {
        self.width <= 0.0 || self.height <= 0.0
    }
    
    /// Get right edge
    pub fn right(&self) -> f32 {
        self.x + self.width
    }
    
    /// Get bottom edge
    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }
    
    /// Get area
    pub fn area(&self) -> f32 {
        self.width * self.height
    }
    
    /// Check if this rectangle intersects another
    pub fn intersects(&self, other: &DirtyRect) -> bool {
        self.x < other.right() &&
        self.right() > other.x &&
        self.y < other.bottom() &&
        self.bottom() > other.y
    }
    
    /// Get intersection with another rectangle
    pub fn intersection(&self, other: &DirtyRect) -> Option<DirtyRect> {
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        
        if left < right && top < bottom {
            Some(DirtyRect::from_bounds(left, top, right, bottom))
        } else {
            None
        }
    }
    
    /// Get union with another rectangle (bounding box)
    pub fn union(&self, other: &DirtyRect) -> DirtyRect {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        
        let left = self.x.min(other.x);
        let top = self.y.min(other.y);
        let right = self.right().max(other.right());
        let bottom = self.bottom().max(other.bottom());
        
        DirtyRect::from_bounds(left, top, right, bottom)
    }
    
    /// Expand rectangle by a margin
    pub fn expand(&self, margin: f32) -> DirtyRect {
        DirtyRect::new(
            self.x - margin,
            self.y - margin,
            self.width + margin * 2.0,
            self.height + margin * 2.0,
        )
    }
    
    /// Convert to integer coordinates (for scissor rect)
    pub fn to_int(&self) -> (i32, i32, i32, i32) {
        (
            self.x.floor() as i32,
            self.y.floor() as i32,
            self.width.ceil() as i32,
            self.height.ceil() as i32,
        )
    }
    
    /// Convert to integer coordinates with upward Y (OpenGL convention)
    pub fn to_gl_scissor(&self, screen_height: u32) -> (i32, i32, i32, i32) {
        let (x, y, w, h) = self.to_int();
        (x, screen_height as i32 - y - h, w, h)
    }
}

impl Default for DirtyRect {
    fn default() -> Self {
        Self::empty()
    }
}

/// Integer rectangle for scissor operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl IntRect {
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self { x, y, width, height }
    }
    
    pub fn from_dirty(rect: &DirtyRect) -> Self {
        let (x, y, w, h) = rect.to_int();
        Self::new(x, y, w, h)
    }
    
    pub fn is_empty(&self) -> bool {
        self.width <= 0 || self.height <= 0
    }
    
    pub fn area(&self) -> i32 {
        self.width * self.height
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_intersection() {
        let a = DirtyRect::new(0.0, 0.0, 100.0, 100.0);
        let b = DirtyRect::new(50.0, 50.0, 100.0, 100.0);
        
        let i = a.intersection(&b).unwrap();
        assert_eq!(i.x, 50.0);
        assert_eq!(i.y, 50.0);
        assert_eq!(i.width, 50.0);
        assert_eq!(i.height, 50.0);
    }
    
    #[test]
    fn test_union() {
        let a = DirtyRect::new(0.0, 0.0, 50.0, 50.0);
        let b = DirtyRect::new(100.0, 100.0, 50.0, 50.0);
        
        let u = a.union(&b);
        assert_eq!(u.x, 0.0);
        assert_eq!(u.y, 0.0);
        assert_eq!(u.width, 150.0);
        assert_eq!(u.height, 150.0);
    }
    
    #[test]
    fn test_no_intersection() {
        let a = DirtyRect::new(0.0, 0.0, 50.0, 50.0);
        let b = DirtyRect::new(100.0, 100.0, 50.0, 50.0);
        
        assert!(a.intersection(&b).is_none());
    }
}
