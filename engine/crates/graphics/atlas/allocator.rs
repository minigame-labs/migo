//! Shelf-based atlas allocator.
//!
//! Pure data structure with no GL dependency.  Given a fixed atlas size
//! (default 2048x2048), packs rectangles into horizontal shelves.
//!
//! When the current atlas page is full a new page is started automatically.
//! The caller is responsible for creating the corresponding GL texture.

/// Maximum texture dimension that will be accepted for atlas packing.
pub const MAX_INPUT_DIM: u16 = 256;

/// Default atlas page size (width and height are always equal).
pub const DEFAULT_ATLAS_SIZE: u16 = 2048;

/// 1-pixel padding between sub-images to avoid sampling bleed.
const PAD: u16 = 1;

/// Describes where a sub-image was placed inside an atlas page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtlasRegion {
    /// Index of the atlas page (0-based).
    pub atlas_id: u32,
    /// X offset in pixels from the left edge of the atlas.
    pub x: u16,
    /// Y offset in pixels from the top edge of the atlas.
    pub y: u16,
    /// Width of the sub-image in pixels.
    pub w: u16,
    /// Height of the sub-image in pixels.
    pub h: u16,
}

/// A horizontal shelf inside one atlas page.
#[derive(Debug)]
struct Shelf {
    /// Y position of the shelf's top edge.
    y: u16,
    /// Fixed height of this shelf (determined by the first image placed).
    height: u16,
    /// Current X cursor -- next image starts here.
    cursor_x: u16,
}

/// One atlas page.
#[derive(Debug)]
struct Page {
    shelves: Vec<Shelf>,
    /// Y position where the next new shelf would be created.
    next_shelf_y: u16,
}

/// Shelf-based atlas allocator.
///
/// Call [`allocate`](AtlasAllocator::allocate) to obtain an [`AtlasRegion`]
/// for each small texture.  The allocator is purely spatial -- it does not
/// touch the GPU.
#[derive(Debug)]
pub struct AtlasAllocator {
    atlas_size: u16,
    pages: Vec<Page>,
}

impl AtlasAllocator {
    /// Create a new allocator with the given square atlas size.
    pub fn new(atlas_size: u16) -> Self {
        Self {
            atlas_size,
            pages: Vec::new(),
        }
    }

    /// Create a new allocator with [`DEFAULT_ATLAS_SIZE`].
    pub fn with_default_size() -> Self {
        Self::new(DEFAULT_ATLAS_SIZE)
    }

    /// Number of atlas pages currently in use.
    pub fn page_count(&self) -> u32 {
        self.pages.len() as u32
    }

    /// The square atlas page size.
    pub fn atlas_size(&self) -> u16 {
        self.atlas_size
    }

    /// Try to allocate a region for a `w * h` sub-image.
    ///
    /// Returns `None` only if `w` or `h` exceeds [`MAX_INPUT_DIM`] or the
    /// atlas size itself (after padding).
    pub fn allocate(&mut self, w: u16, h: u16) -> Option<AtlasRegion> {
        if w == 0 || h == 0 || w > MAX_INPUT_DIM || h > MAX_INPUT_DIM {
            return None;
        }

        // Padded dimensions that the shelf must accommodate.
        let pw = w.checked_add(PAD)?;
        let ph = h.checked_add(PAD)?;
        if pw > self.atlas_size || ph > self.atlas_size {
            return None;
        }

        // 1. Try to fit into an existing shelf on an existing page.
        for (page_idx, page) in self.pages.iter_mut().enumerate() {
            if let Some(region) =
                try_fit_in_page(page, self.atlas_size, page_idx as u32, w, h, pw, ph)
            {
                return Some(region);
            }
        }

        // 2. No existing page could fit it -- start a new page.
        let page_idx = self.pages.len() as u32;
        let mut page = Page {
            shelves: Vec::new(),
            next_shelf_y: 0,
        };
        let region = try_fit_in_page(&mut page, self.atlas_size, page_idx, w, h, pw, ph);
        self.pages.push(page);
        // A freshly created page can always hold one sub-image (checked above).
        region
    }

    /// Reset all allocations.  The caller is responsible for deleting any
    /// GL textures that were associated with the previous pages.
    pub fn clear(&mut self) {
        self.pages.clear();
    }
}

/// Try to place `w x h` (padded to `pw x ph`) into `page`.
fn try_fit_in_page(
    page: &mut Page,
    atlas_size: u16,
    page_idx: u32,
    w: u16,
    h: u16,
    pw: u16,
    ph: u16,
) -> Option<AtlasRegion> {
    // Scan existing shelves for one with enough remaining width *and* whose
    // fixed height is >= the padded height of the incoming image.
    for shelf in page.shelves.iter_mut() {
        if shelf.height >= ph && (atlas_size - shelf.cursor_x) >= pw {
            let region = AtlasRegion {
                atlas_id: page_idx,
                x: shelf.cursor_x,
                y: shelf.y,
                w,
                h,
            };
            shelf.cursor_x += pw;
            return Some(region);
        }
    }

    // No shelf fits -- try to open a new shelf.
    let remaining_y = atlas_size.checked_sub(page.next_shelf_y)?;
    if remaining_y < ph {
        return None;
    }

    let shelf_y = page.next_shelf_y;
    page.shelves.push(Shelf {
        y: shelf_y,
        height: ph,
        cursor_x: pw,
    });
    page.next_shelf_y = shelf_y + ph;

    Some(AtlasRegion {
        atlas_id: page_idx,
        x: 0,
        y: shelf_y,
        w,
        h,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_allocation() {
        let mut alloc = AtlasAllocator::new(512);
        let r = alloc.allocate(64, 64).expect("should allocate");
        assert_eq!(r.atlas_id, 0);
        assert_eq!(r.x, 0);
        assert_eq!(r.y, 0);
        assert_eq!(r.w, 64);
        assert_eq!(r.h, 64);
    }

    #[test]
    fn same_shelf() {
        let mut alloc = AtlasAllocator::new(512);
        let r1 = alloc.allocate(100, 50).unwrap();
        let r2 = alloc.allocate(80, 40).unwrap();
        // Both should land on the same shelf (shelf height = 50+1 = 51, 40+1 fits).
        assert_eq!(r1.y, r2.y);
        // r2 starts right after r1 + padding.
        assert_eq!(r2.x, 100 + PAD);
    }

    #[test]
    fn new_shelf_when_width_full() {
        let mut alloc = AtlasAllocator::new(256);
        // Fill first shelf.
        let r1 = alloc.allocate(200, 50).unwrap();
        // Next one does not fit horizontally -> new shelf.
        let r2 = alloc.allocate(200, 50).unwrap();
        assert_eq!(r1.atlas_id, r2.atlas_id);
        assert_ne!(r1.y, r2.y);
    }

    #[test]
    fn new_page_when_height_full() {
        let mut alloc = AtlasAllocator::new(128);
        let r1 = alloc.allocate(64, 64).unwrap();
        // First shelf takes 64+1 = 65px.  Second shelf another 65.  128 - 65 = 63 < 65 -> new page.
        let _ = alloc.allocate(64, 64).unwrap();
        assert_eq!(alloc.page_count(), 2);
        assert_eq!(r1.atlas_id, 0);
    }

    #[test]
    fn rejects_oversized() {
        let mut alloc = AtlasAllocator::with_default_size();
        assert!(alloc.allocate(MAX_INPUT_DIM + 1, 10).is_none());
        assert!(alloc.allocate(10, MAX_INPUT_DIM + 1).is_none());
        assert!(alloc.allocate(0, 10).is_none());
        assert!(alloc.allocate(10, 0).is_none());
    }

    #[test]
    fn many_small_textures() {
        let mut alloc = AtlasAllocator::new(DEFAULT_ATLAS_SIZE);
        // Pack a bunch of 32x32 tiles.  2048 / (32+1) = 62 per row,
        // 2048 / (32+1) = 62 rows -> ~3844 per page.
        for _ in 0..3844 {
            let r = alloc.allocate(32, 32);
            assert!(r.is_some());
        }
        // Should be on page 0 still (3844 fit in a 2048x2048 atlas).
        assert_eq!(alloc.page_count(), 1);
    }

    #[test]
    fn clear_resets() {
        let mut alloc = AtlasAllocator::new(512);
        alloc.allocate(100, 100).unwrap();
        assert_eq!(alloc.page_count(), 1);
        alloc.clear();
        assert_eq!(alloc.page_count(), 0);
    }
}
