use femtovg::{Canvas as FvCanvas, FontId, renderer::OpenGl};
use std::{
    collections::HashMap,
    fs,
    sync::{Arc, OnceLock, RwLock},
};
use tracing::{debug, info, warn};

use shared::error::{EngineError, EngineResult, ErrorCode};

#[inline]
fn ee(code: ErrorCode, detail: impl Into<String>) -> EngineError {
    EngineError::from_detail(code, detail)
}

/// Font data stored globally (bytes are shared by Arc).
#[derive(Clone)]
pub(crate) struct FontData {
    pub name: String,
    pub bytes: Arc<Vec<u8>>,
}

pub(crate) struct GlobalFontStore {
    fonts_by_key: HashMap<String, FontData>,
    default_key: String,
}

impl GlobalFontStore {
    fn new() -> Self {
        Self {
            fonts_by_key: HashMap::new(),
            default_key: "sans-serif".to_string(),
        }
    }

    pub(crate) fn insert(&mut self, key: &str, data: FontData) {
        self.fonts_by_key.insert(key.to_string(), data);
    }

    fn set_default_key(&mut self, key: &str) {
        self.default_key = key.to_string();
    }

    fn get(&self, key: &str) -> Option<&FontData> {
        self.fonts_by_key.get(key)
    }

    fn iter(&self) -> impl Iterator<Item = (&String, &FontData)> {
        self.fonts_by_key.iter()
    }

    fn default_data(&self) -> Option<&FontData> {
        self.fonts_by_key.get(&self.default_key)
    }
}

/// Singleton global font store (behind RwLock for dynamic font loading).
static GLOBAL_FONTS: OnceLock<RwLock<GlobalFontStore>> = OnceLock::new();

/// Initialize/load the global font store once.
///
/// Strategy:
/// - Prefer CJK-capable NotoSansSC-Regular as default if present.
/// - Otherwise use Roboto-Regular.
/// - Load common mappings and ignore missing files.
fn init_global_fonts() -> GlobalFontStore {
    let mut store = GlobalFontStore::new();

    // (path, key)
    // Keys are "logical family keys" used by Canvas/CSS-ish parsing.
    let font_mappings: [(&str, &str); 9] = [
        ("/system/fonts/Roboto-Regular.ttf", "sans-serif"),
        ("/system/fonts/Roboto-Bold.ttf", "sans-serif-bold"),
        ("/system/fonts/Roboto-Italic.ttf", "sans-serif-italic"),
        (
            "/system/fonts/Roboto-BoldItalic.ttf",
            "sans-serif-bold-italic",
        ),
        ("/system/fonts/NotoSansSC-Bold.ttf", "sans-serif-sc-bold"),
        ("/system/fonts/NotoSansSC-Light.ttf", "sans-serif-sc-light"),
        ("/system/fonts/RobotoMono-Regular.ttf", "monospace"),
        ("/system/fonts/NotoSans-Regular.ttf", "noto-sans"),
        ("/system/fonts/NotoSerif-Regular.ttf", "serif"),
    ];

    // Also attempt to load a good default CJK font (Regular).
    let cjk_default_candidates = [
        ("/system/fonts/NotoSansSC-Regular.ttf", "sans-serif-sc"),
        ("/system/fonts/NotoSansCJK-Regular.ttc", "sans-serif-cjk"),
        ("/system/fonts/NotoSansCJKsc-Regular.otf", "sans-serif-cjk"),
    ];

    // 1) Load CJK default candidates first (so we can prefer them as default).
    for (path, key) in cjk_default_candidates {
        if let Ok(bytes) = fs::read(path) {
            let data = FontData {
                name: key.to_string(),
                bytes: Arc::new(bytes),
            };
            store.insert(key, data);
            // Prefer first successful candidate as default.
            store.set_default_key(key);
            info!("Loaded system font (default): {} as {}", path, key);
            break;
        }
    }

    // 2) Load normal mappings.
    for (path, key) in font_mappings {
        match fs::read(path) {
            Ok(bytes) => {
                let data = FontData {
                    name: key.to_string(),
                    bytes: Arc::new(bytes),
                };
                store.insert(key, data);
                info!("Loaded system font: {} as {}", path, key);
            }
            Err(e) => {
                debug!("Font file not found/readable: {} ({})", path, e);
            }
        }
    }

    // 3) Ensure we have *some* default.
    if store.default_data().is_none() {
        if store.get("sans-serif").is_some() {
            store.set_default_key("sans-serif");
        } else {
            let first_key: Option<String> = store.iter().next().map(|(k, _)| k.clone());
            if let Some(first_key) = first_key {
                store.set_default_key(&first_key);
            }
        }
    }

    if store.default_data().is_none() {
        warn!("No usable system fonts found in /system/fonts. Text rendering will likely fail.");
    } else {
        info!("Global default font key = {}", store.default_key);
    }

    store
}

/// Get global font store (lazy initialized, read lock).
pub(crate) fn global_fonts() -> std::sync::RwLockReadGuard<'static, GlobalFontStore> {
    GLOBAL_FONTS
        .get_or_init(|| RwLock::new(init_global_fonts()))
        .read()
        .unwrap()
}

/// Get global font store (write lock, for dynamic font loading).
pub(crate) fn global_fonts_mut() -> std::sync::RwLockWriteGuard<'static, GlobalFontStore> {
    GLOBAL_FONTS
        .get_or_init(|| RwLock::new(init_global_fonts()))
        .write()
        .unwrap()
}

/// FontManager is per-canvas:
/// it registers global font bytes into this canvas, creating local FontId values.
pub(crate) struct FontManager {
    font_ids_by_key: HashMap<String, FontId>,
    default_font_id: Option<FontId>,
}

impl FontManager {
    /// Create a per-canvas FontManager and register all globally loaded fonts.
    pub(crate) fn new(canvas: &mut FvCanvas<OpenGl>) -> EngineResult<Self> {
        let store = global_fonts();

        let mut font_ids_by_key = HashMap::new();
        for (key, data) in store.iter() {
            match canvas.add_font_mem(&data.bytes) {
                Ok(fid) => {
                    font_ids_by_key.insert(key.clone(), fid);
                    debug!("Registered font '{}' as FontId {:?}", key, fid);
                }
                Err(e) => {
                    warn!("Failed to register font '{}' to canvas: {}", key, e);
                }
            }
        }

        let default_font_id = store
            .default_data()
            .and_then(|d| font_ids_by_key.get(&d.name).copied())
            .or_else(|| font_ids_by_key.get("sans-serif").copied())
            .or_else(|| font_ids_by_key.values().next().copied());

        if default_font_id.is_none() {
            warn!("FontManager: no default font id resolved for this canvas");
        }

        Ok(Self {
            font_ids_by_key,
            default_font_id,
        })
    }

    /// Return the default FontId for this canvas (if any).
    pub(crate) fn default_font_id(&self) -> Option<FontId> {
        self.default_font_id
    }

    /// Resolve a FontId given a CSS-ish family + style flags.
    ///
    /// Rules:
    /// - Try exact styled variants: "<family>-bold-italic", "<family>-bold", "<family>-italic"
    /// - Try plain "<family>"
    /// - Common aliases:
    ///   * "sans-serif" -> "sans-serif"
    ///   * "serif"      -> "serif"
    ///   * "monospace"  -> "monospace"
    /// - Fallback to CJK-capable default key if present (global default), then "sans-serif", then any.
    pub(crate) fn resolve_font_id(&self, family: &str, bold: bool, italic: bool) -> Option<FontId> {
        // Normalize family:
        let fam = normalize_family_key(family);

        // Try styled variants first.
        if bold && italic {
            if let Some(id) = self.font_ids_by_key.get(&format!("{}-bold-italic", fam)) {
                return Some(*id);
            }
        }
        if bold {
            if let Some(id) = self.font_ids_by_key.get(&format!("{}-bold", fam)) {
                return Some(*id);
            }
        }
        if italic {
            if let Some(id) = self.font_ids_by_key.get(&format!("{}-italic", fam)) {
                return Some(*id);
            }
        }

        // Try base family.
        if let Some(id) = self.font_ids_by_key.get(&fam) {
            return Some(*id);
        }

        // If requested family is sans-serif, also try our CJK sans-serif default key if present.
        // This helps when font string uses only "sans-serif" but text contains CJK.
        if fam == "sans-serif" {
            // global default key might be "sans-serif-sc" or similar.
            let default_key = &global_fonts().default_key;
            if let Some(id) = self.font_ids_by_key.get(default_key) {
                return Some(*id);
            }
            // or try explicit sc
            if let Some(id) = self.font_ids_by_key.get("sans-serif-sc") {
                return Some(*id);
            }
        }

        // Fallback: global default, then "sans-serif", then any.
        let default_key = &global_fonts().default_key;
        self.font_ids_by_key
            .get(default_key)
            .copied()
            .or_else(|| self.font_ids_by_key.get("sans-serif").copied())
            .or_else(|| self.default_font_id)
    }

    /// Parse a CSS-ish `font` shorthand (very small subset) and return:
    /// (families_in_order, size_px, bold, italic)
    ///
    /// Examples supported:
    /// - "bold italic 16px sans-serif"
    /// - "16px 'Roboto' , sans-serif"
    /// - "italic 14px monospace"
    pub(crate) fn parse_font_shorthand(
        &self,
        font: &str,
    ) -> (Vec<String>, Option<f32>, bool, bool) {
        let tokens = tokenize_font(font);

        let mut bold = false;
        let mut italic = false;
        let mut size_px: Option<f32> = None;

        let mut family_tokens: Vec<String> = Vec::new();
        let mut after_size = false;

        for t in tokens {
            let tl = t.to_lowercase();

            if !after_size {
                if tl == "bold" {
                    bold = true;
                    continue;
                }
                if tl == "italic" {
                    italic = true;
                    continue;
                }
                if tl == "normal" {
                    continue;
                }

                if let Some(px) = parse_px_size(&tl) {
                    size_px = Some(px);
                    after_size = true;
                    continue;
                }

                continue;
            } else {
                family_tokens.push(t);
            }
        }

        let families = parse_family_list(&family_tokens);
        (families, size_px, bold, italic)
    }

    pub(crate) fn resolve_from_font_string(&self, font: &str) -> (Option<FontId>, Option<f32>) {
        let (families, size_px, bold, italic) = self.parse_font_shorthand(font);
        for fam in families {
            if let Some(id) = self.resolve_font_id(&fam, bold, italic) {
                return (Some(id), size_px);
            }
        }
        (self.default_font_id(), size_px)
    }
}

impl FontManager {
    /// Register a single font (already in GlobalFontStore) into this canvas.
    /// Returns the FontId if successful.
    pub(crate) fn register_font(
        &mut self,
        canvas: &mut FvCanvas<OpenGl>,
        key: &str,
        data: &FontData,
    ) -> Option<FontId> {
        if self.font_ids_by_key.contains_key(key) {
            return self.font_ids_by_key.get(key).copied();
        }
        match canvas.add_font_mem(&data.bytes) {
            Ok(fid) => {
                self.font_ids_by_key.insert(key.to_string(), fid);
                debug!("Registered dynamic font '{}' as FontId {:?}", key, fid);
                Some(fid)
            }
            Err(e) => {
                warn!("Failed to register dynamic font '{}' to canvas: {}", key, e);
                None
            }
        }
    }

    pub(crate) fn parse_font_string(&self, font: &str) -> (String, Option<f32>, bool, bool) {
        let (families, size, bold, italic) = self.parse_font_shorthand(font);
        let fam = families
            .into_iter()
            .next()
            .unwrap_or_else(|| "sans-serif".to_string());
        (fam, size, bold, italic)
    }

    pub(crate) fn get_font_id_with_style(
        &self,
        family: &str,
        bold: bool,
        italic: bool,
    ) -> Option<FontId> {
        self.resolve_font_id(family, bold, italic)
    }

    pub(crate) fn get_default_font_id(&self) -> Option<FontId> {
        self.default_font_id()
    }
}

fn normalize_family_key(family: &str) -> String {
    let mut f = family.trim().to_lowercase();
    // Strip quotes if present.
    if (f.starts_with('"') && f.ends_with('"')) || (f.starts_with('\'') && f.ends_with('\'')) {
        f = f[1..f.len() - 1].to_string();
    }

    match f.as_str() {
        "sans" | "sans serif" | "sans-serif" => "sans-serif".to_string(),
        "serif" => "serif".to_string(),
        "mono" | "monospace" => "monospace".to_string(),
        // Common explicit mapping: if user says "default" treat as sans-serif.
        "default" => "sans-serif".to_string(),
        other => other.to_string(),
    }
}

fn parse_px_size(token: &str) -> Option<f32> {
    // Supports "16px" or "16.5px".
    if let Some(num) = token.strip_suffix("px") {
        return num.parse::<f32>().ok().map(|v| v.max(0.5));
    }

    // Support "16px/1.2" (we ignore line-height)
    if let Some((num, _rest)) = token.split_once("px/") {
        return num.parse::<f32>().ok().map(|v| v.max(0.5));
    }

    None
}

fn tokenize_font(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut quote: Option<char> = None;

    for ch in s.chars() {
        match quote {
            Some(q) => {
                if ch == q {
                    quote = None;
                    buf.push(ch);
                } else {
                    buf.push(ch);
                }
            }
            None => {
                if ch == '\'' || ch == '"' {
                    quote = Some(ch);
                    buf.push(ch);
                } else if ch.is_whitespace() {
                    if !buf.trim().is_empty() {
                        out.push(buf.trim().to_string());
                    }
                    buf.clear();
                } else {
                    buf.push(ch);
                }
            }
        }
    }

    if !buf.trim().is_empty() {
        out.push(buf.trim().to_string());
    }
    out
}

fn parse_family_list(tokens: &[String]) -> Vec<String> {
    if tokens.is_empty() {
        return vec!["sans-serif".to_string()];
    }

    let joined = tokens.join(" ");

    let mut families = Vec::new();
    let mut buf = String::new();
    let mut quote: Option<char> = None;

    for ch in joined.chars() {
        match quote {
            Some(q) => {
                if ch == q {
                    quote = None;
                }
                buf.push(ch);
            }
            None => {
                if ch == '\'' || ch == '"' {
                    quote = Some(ch);
                    buf.push(ch);
                } else if ch == ',' {
                    let f = buf.trim();
                    if !f.is_empty() {
                        families.push(f.to_string());
                    }
                    buf.clear();
                } else {
                    buf.push(ch);
                }
            }
        }
    }

    let f = buf.trim();
    if !f.is_empty() {
        families.push(f.to_string());
    }

    if families.is_empty() {
        families.push("sans-serif".to_string());
    }

    families
}
