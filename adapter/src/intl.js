// Minimal `Intl` polyfill. Migo's V8 is built no-i18n (no ICU) to keep the .so
// small, so the standard `Intl` global is absent — but browser engines assume it
// exists (e.g. PixiJS v8 text metrics reference `Intl` during init and throw a
// ReferenceError without it). This provides just enough surface to keep engines
// running; it is NOT a full ICU implementation (Segmenter splits by code point,
// formatters stringify). Extend as games need more.
//
// index.js only publishes this when `globalThis.Intl` is absent, so a future
// ICU-enabled build is never clobbered.

function segment(str) {
  str = String(str);
  const parts = Array.from(str); // code points, not UTF-16 units
  return {
    [Symbol.iterator]() {
      let i = 0;
      return {
        next() {
          if (i < parts.length) {
            const value = { segment: parts[i], index: i, input: str, isWordLike: true };
            i++;
            return { value, done: false };
          }
          return { value: undefined, done: true };
        },
      };
    },
    containing(index = 0) {
      const i = Math.max(0, Math.min(parts.length - 1, index | 0));
      return { segment: parts[i] || "", index: i, input: str };
    },
  };
}

const Intl = {
  Segmenter: class {
    segment(input) {
      return segment(input);
    }
    resolvedOptions() {
      return {};
    }
  },
  NumberFormat: class {
    format(n) {
      return String(n);
    }
    formatToParts(n) {
      return [{ type: "integer", value: String(n) }];
    }
    resolvedOptions() {
      return {};
    }
  },
  DateTimeFormat: class {
    format(d) {
      return String(d);
    }
    formatToParts() {
      return [];
    }
    resolvedOptions() {
      return { timeZone: "UTC" };
    }
  },
  Collator: class {
    compare(a, b) {
      return a < b ? -1 : a > b ? 1 : 0;
    }
    resolvedOptions() {
      return {};
    }
  },
  PluralRules: class {
    select() {
      return "other";
    }
    resolvedOptions() {
      return {};
    }
  },
  getCanonicalLocales(locales) {
    return [].concat(locales || []);
  },
};

export default Intl;
