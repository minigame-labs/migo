/// Lightweight RGBA color type for the render protocol.
///
/// Replaces `femtovg::Color` in the shared crate to avoid pulling in the heavy
/// `femtovg` dependency just for color values in [`Canvas2DCmd::SetFillStyle`]
/// and [`Canvas2DCmd::SetStrokeStyle`].
///
/// All components are stored as normalized `f32` values in `[0.0, 1.0]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    /// Create a color from floating-point RGBA components (each in `[0.0, 1.0]`).
    #[inline]
    pub fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// Create a color from integer RGBA components (each in `[0, 255]`).
    #[inline]
    pub fn rgbai(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a: a as f32 / 255.0,
        }
    }

    /// Create an opaque color from integer RGB components (each in `[0, 255]`).
    #[inline]
    pub fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self::rgbai(r, g, b, 255)
    }

    /// Parse a CSS hex color string (`#RGB`, `#RGBA`, `#RRGGBB`, or `#RRGGBBAA`).
    ///
    /// Returns [`Color::black()`] on parse failure.
    pub fn hex(s: &str) -> Self {
        let s = s.strip_prefix('#').unwrap_or(s);
        match s.len() {
            // #RGB
            3 => {
                let r = u8_from_hex_char(s.as_bytes()[0]);
                let g = u8_from_hex_char(s.as_bytes()[1]);
                let b = u8_from_hex_char(s.as_bytes()[2]);
                Self::rgbai(r << 4 | r, g << 4 | g, b << 4 | b, 255)
            }
            // #RGBA
            4 => {
                let r = u8_from_hex_char(s.as_bytes()[0]);
                let g = u8_from_hex_char(s.as_bytes()[1]);
                let b = u8_from_hex_char(s.as_bytes()[2]);
                let a = u8_from_hex_char(s.as_bytes()[3]);
                Self::rgbai(r << 4 | r, g << 4 | g, b << 4 | b, a << 4 | a)
            }
            // #RRGGBB
            6 => {
                let r = u8_from_hex_pair(s.as_bytes()[0], s.as_bytes()[1]);
                let g = u8_from_hex_pair(s.as_bytes()[2], s.as_bytes()[3]);
                let b = u8_from_hex_pair(s.as_bytes()[4], s.as_bytes()[5]);
                Self::rgbai(r, g, b, 255)
            }
            // #RRGGBBAA
            8 => {
                let r = u8_from_hex_pair(s.as_bytes()[0], s.as_bytes()[1]);
                let g = u8_from_hex_pair(s.as_bytes()[2], s.as_bytes()[3]);
                let b = u8_from_hex_pair(s.as_bytes()[4], s.as_bytes()[5]);
                let a = u8_from_hex_pair(s.as_bytes()[6], s.as_bytes()[7]);
                Self::rgbai(r, g, b, a)
            }
            _ => Self::black(),
        }
    }

    #[inline]
    pub fn white() -> Self {
        Self { r: 1.0, g: 1.0, b: 1.0, a: 1.0 }
    }

    #[inline]
    pub fn black() -> Self {
        Self { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }
    }

    #[inline]
    pub fn transparent() -> Self {
        Self { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }
    }
}

#[inline]
fn u8_from_hex_char(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => 0,
    }
}

#[inline]
fn u8_from_hex_pair(hi: u8, lo: u8) -> u8 {
    u8_from_hex_char(hi) << 4 | u8_from_hex_char(lo)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rgb() {
        let c = Color::rgb(255, 128, 0);
        assert_eq!(c.r, 1.0);
        assert!((c.g - 128.0 / 255.0).abs() < f32::EPSILON);
        assert_eq!(c.b, 0.0);
        assert_eq!(c.a, 1.0);
    }

    #[test]
    fn test_rgbai() {
        let c = Color::rgbai(0, 0, 0, 0);
        assert_eq!(c, Color::transparent());
    }

    #[test]
    fn test_hex_6() {
        let c = Color::hex("#FF8000");
        assert_eq!(c.r, 1.0);
        assert!((c.g - 128.0 / 255.0).abs() < f32::EPSILON);
        assert_eq!(c.b, 0.0);
        assert_eq!(c.a, 1.0);
    }

    #[test]
    fn test_hex_3() {
        let c = Color::hex("#F00");
        assert_eq!(c.r, 1.0);
        assert_eq!(c.g, 0.0);
        assert_eq!(c.b, 0.0);
        assert_eq!(c.a, 1.0);
    }

    #[test]
    fn test_hex_8() {
        let c = Color::hex("#FF000080");
        assert_eq!(c.r, 1.0);
        assert_eq!(c.g, 0.0);
        assert_eq!(c.b, 0.0);
        assert!((c.a - 128.0 / 255.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_black_white() {
        assert_eq!(Color::black(), Color::rgb(0, 0, 0));
        assert_eq!(Color::white(), Color::rgb(255, 255, 255));
    }
}
