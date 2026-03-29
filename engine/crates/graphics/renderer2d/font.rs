use femtovg::{renderer::OpenGl, Canvas as FvCanvas, FontId};
use std::{
    collections::{HashMap, HashSet},
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
/// - Keep a minimal built-in set (sans-serif + CJK fallback + optional serif/monospace).
/// - Prefer sans-serif as default (closer to Canvas/Arial behavior).
/// - Prefer SC-specific CJK faces before generic TTC fallback.
fn init_global_fonts() -> GlobalFontStore {
    let mut store = GlobalFontStore::new();

    fn load_first_existing(store: &mut GlobalFontStore, key: &str, candidates: &[&str]) -> bool {
        for path in candidates {
            match fs::read(path) {
                Ok(bytes) => {
                    let data = FontData {
                        name: key.to_string(),
                        bytes: Arc::new(bytes),
                    };
                    store.insert(key, data);
                    info!("Loaded system font: {} as {}", path, key);
                    return true;
                }
                Err(e) => {
                    debug!("Font file not found/readable: {} ({})", path, e);
                }
            }
        }
        false
    }

    // Load only essential defaults to reduce startup overhead.
    // Extra fonts should come from dynamic loadFont() when needed.
    let loaded_sans = load_first_existing(
        &mut store,
        "sans-serif",
        &[
            "/system/fonts/Roboto-Regular.ttf",
            "/system/fonts/NotoSans-Regular.ttf",
            "/system/fonts/DroidSans.ttf",
        ],
    );

    let loaded_sc = load_first_existing(
        &mut store,
        "sans-serif-sc",
        &[
            "/system/fonts/NotoSansSC-Regular.ttf",
            "/system/fonts/NotoSansSC-Regular.otf",
            "/system/fonts/NotoSansCJKsc-Regular.otf",
        ],
    );

    if !loaded_sc {
        // Last-resort CJK fallback; TTC face 0 may not always be SC.
        let _ = load_first_existing(
            &mut store,
            "sans-serif-cjk",
            &[
                "/system/fonts/NotoSansCJK-Regular.ttc",
                "/system/fonts/DroidSansFallback.ttf",
            ],
        );
    }

    let _ = load_first_existing(
        &mut store,
        "serif",
        &["/system/fonts/NotoSerif-Regular.ttf"],
    );

    let _ = load_first_existing(
        &mut store,
        "monospace",
        &[
            "/system/fonts/RobotoMono-Regular.ttf",
            "/system/fonts/DroidSansMono.ttf",
        ],
    );

    // Ensure we have *some* default.
    // Prefer sans-serif to match Canvas/Arial expectations.
    if loaded_sans {
        store.set_default_key("sans-serif");
    } else if store.get("sans-serif-sc").is_some() {
        store.set_default_key("sans-serif-sc");
    } else if store.get("sans-serif-cjk").is_some() {
        store.set_default_key("sans-serif-cjk");
    }

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

        // Register fonts in deterministic priority order so fallback is stable.
        const PRIORITY_KEYS: [&str; 5] = [
            "sans-serif",
            "sans-serif-sc",
            "sans-serif-cjk",
            "serif",
            "monospace",
        ];

        let mut register_order = Vec::new();
        let mut seen = HashSet::new();

        for key in PRIORITY_KEYS {
            if store.get(key).is_some() {
                register_order.push(key.to_string());
                seen.insert(key.to_string());
            }
        }

        let mut extras: Vec<String> = store
            .iter()
            .map(|(k, _)| k.clone())
            .filter(|k| !seen.contains(k))
            .collect();
        extras.sort_unstable();
        register_order.extend(extras);

        for key in register_order {
            let Some(data) = store.get(&key) else {
                continue;
            };

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

        let default_font_id = font_ids_by_key
            .get("sans-serif")
            .copied()
            .or_else(|| {
                store
                    .default_data()
                    .and_then(|d| font_ids_by_key.get(&d.name).copied())
            })
            .or_else(|| font_ids_by_key.get("sans-serif-sc").copied())
            .or_else(|| font_ids_by_key.get("sans-serif-cjk").copied())
            .or_else(|| font_ids_by_key.get("serif").copied())
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
    /// - Fallback order: sans-serif -> explicit CJK -> global default -> any.
    pub(crate) fn resolve_font_id(&self, family: &str, bold: bool, italic: bool) -> Option<FontId> {
        // Normalize family:
        let fam = normalize_family_key(family);

        // Reuse a stack buffer for styled key lookups (avoids format! heap allocs).
        let mut key_buf = String::with_capacity(64);

        // Try styled variants first.
        if bold && italic {
            key_buf.clear();
            key_buf.push_str(&fam);
            key_buf.push_str("-bold-italic");
            if let Some(id) = self.font_ids_by_key.get(&key_buf) {
                return Some(*id);
            }
        }
        if bold {
            key_buf.clear();
            key_buf.push_str(&fam);
            key_buf.push_str("-bold");
            if let Some(id) = self.font_ids_by_key.get(&key_buf) {
                return Some(*id);
            }
        }
        if italic {
            key_buf.clear();
            key_buf.push_str(&fam);
            key_buf.push_str("-italic");
            if let Some(id) = self.font_ids_by_key.get(&key_buf) {
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
            // Prefer explicit SC/CJK fallbacks for Chinese text.
            if let Some(id) = self.font_ids_by_key.get("sans-serif-sc") {
                return Some(*id);
            }
            if let Some(id) = self.font_ids_by_key.get("sans-serif-cjk") {
                return Some(*id);
            }
            let default_key = &global_fonts().default_key;
            if let Some(id) = self.font_ids_by_key.get(default_key) {
                return Some(*id);
            }
        }

        // Fallback: sans-serif first, then explicit CJK, then global default.
        let default_key = &global_fonts().default_key;
        self.font_ids_by_key
            .get("sans-serif")
            .copied()
            .or_else(|| self.font_ids_by_key.get("sans-serif-sc").copied())
            .or_else(|| self.font_ids_by_key.get("sans-serif-cjk").copied())
            .or_else(|| self.font_ids_by_key.get(default_key).copied())
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
        "arial" | "arialmt" | "arial-boldmt" | "arial-bold" | "helvetica" | "helvetica-bold"
        | "helvetica neue" => "sans-serif".to_string(),
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
