use std::collections::{HashMap, VecDeque};

const MAX_TEXT_LAYOUT_CACHE_ENTRIES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TextLayoutKey {
    text: String,
    font: String,
    max_width_bits: u32,
}

impl TextLayoutKey {
    pub fn new(text: &str, font: &str, max_width: f32) -> Self {
        Self {
            text: text.to_string(),
            font: font.to_string(),
            max_width_bits: max_width.to_bits(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CachedTextLayout {
    pub width: f32,
    pub height: f32,
}

pub struct TextLayoutCache {
    layouts: HashMap<TextLayoutKey, CachedTextLayout>,
    insertion_order: VecDeque<TextLayoutKey>,
}

impl TextLayoutCache {
    pub fn new() -> Self {
        Self {
            layouts: HashMap::new(),
            insertion_order: VecDeque::new(),
        }
    }

    pub fn get(&self, key: &TextLayoutKey) -> Option<&CachedTextLayout> {
        self.layouts.get(key)
    }

    pub fn insert(&mut self, key: TextLayoutKey, layout: CachedTextLayout) {
        if self.layouts.contains_key(&key) {
            self.layouts.insert(key, layout);
            return;
        }

        if self.layouts.len() >= MAX_TEXT_LAYOUT_CACHE_ENTRIES {
            while let Some(oldest) = self.insertion_order.pop_front() {
                if self.layouts.remove(&oldest).is_some() {
                    break;
                }
            }
        }

        self.insertion_order.push_back(key.clone());
        self.layouts.insert(key, layout);
    }

    pub fn clear(&mut self) {
        self.layouts.clear();
        self.insertion_order.clear();
    }
}

impl Default for TextLayoutCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_changes_when_font_or_content_changes() {
        let a = TextLayoutKey::new("hello", "16px Sans", 200.0);
        let b = TextLayoutKey::new("hello", "18px Sans", 200.0);
        let c = TextLayoutKey::new("world", "16px Sans", 200.0);

        assert_ne!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn clearing_cache_removes_existing_layouts() {
        let key = TextLayoutKey::new("hello", "16px Sans", 200.0);
        let mut cache = TextLayoutCache::new();
        cache.insert(
            key.clone(),
            CachedTextLayout {
                width: 42.0,
                height: 18.0,
            },
        );

        assert!(cache.get(&key).is_some());

        cache.clear();

        assert!(cache.get(&key).is_none());
    }

    #[test]
    fn cache_evicts_oldest_entries_when_capacity_is_exceeded() {
        const CAPACITY: usize = 256;

        let mut cache = TextLayoutCache::new();

        for idx in 0..=CAPACITY {
            let key = TextLayoutKey::new(&format!("text-{idx}"), "16px Sans", 200.0);
            cache.insert(
                key,
                CachedTextLayout {
                    width: idx as f32,
                    height: 18.0,
                },
            );
        }

        let oldest = TextLayoutKey::new("text-0", "16px Sans", 200.0);
        let newest = TextLayoutKey::new(&format!("text-{CAPACITY}"), "16px Sans", 200.0);

        assert!(cache.get(&oldest).is_none());
        assert!(cache.get(&newest).is_some());
    }
}
