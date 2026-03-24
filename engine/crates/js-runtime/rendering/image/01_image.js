import { primordials } from "ext:core/mod.js";
import {
    op_create_image,
    op_load_image,
    op_destroy_image,
    op_preload_images,
    op_clear_image_cache,
    op_get_image_cache_stats
} from "ext:core/ops";

const { SafeFinalizationRegistry } = primordials;

const registry = new SafeFinalizationRegistry((rid) => {
    try {
        op_destroy_image(rid);
    } catch (_) { }
});

class Image {
    constructor() {
        this._src = "";
        this.width = 0;
        this.height = 0;

        this._onload = null;
        this._onerror = null;
        this._listeners = { load: [], error: [] };

        this._loaded = false;
        this._error = null;

        // "caller image id" (alias). Rust cache uses this for alias/ref tracking.
        this._rid = op_create_image();

        // "shared image id" (the actual underlying shared resource id)
        // If not loaded yet, fall back to _rid.
        this._shared_img_id = this._rid;

        // prevent out-of-order loads from stomping state
        this._load_seq = 0;

        // token used for unregister if needed
        this._finalize_token = {};
        registry.register(this, this._rid, this._finalize_token);
    }

    // For drawImage: prefer shared id if available.
    get rid() {
        return this._shared_img_id ?? this._rid;
    }

    get loaded() {
        return this._loaded;
    }

    get error() {
        return this._error;
    }

    get src() {
        return this._src;
    }

    set src(url) {
        this._src = String(url ?? "");
        this._startLoad(this._src);
    }

    set onload(fn) {
        this._onload = typeof fn === "function" ? fn : null;
    }

    set onerror(fn) {
        this._onerror = typeof fn === "function" ? fn : null;
    }

    addEventListener(type, fn) {
        if (typeof fn !== "function") return;
        const list = this._listeners[type];
        if (list && list.indexOf(fn) === -1) list.push(fn);
    }

    removeEventListener(type, fn) {
        const list = this._listeners[type];
        if (!list) return;
        const i = list.indexOf(fn);
        if (i !== -1) list.splice(i, 1);
    }

    _fireListeners(type, arg) {
        const list = this._listeners[type];
        for (let i = 0; i < list.length; i++) {
            try { list[i](arg); } catch (_) {}
        }
    }

    _startLoad(url) {
        const seq = ++this._load_seq;

        // reset observable state like browsers do (roughly)
        this._loaded = false;
        this._error = null;
        this.width = 0;
        this.height = 0;

        // Empty src: treat as error (browsers treat as a request to current document; for your runtime we error)
        if (!url) {
            const err = new TypeError("Image.src is empty");
            this._error = err;
            this._onerror && this._onerror(err);
            this._fireListeners('error', err);
            return;
        }

        op_load_image(this._rid, url)
            .then((dim) => {
                // out-of-order: ignore if a newer src has been set
                if (seq !== this._load_seq) return;

                const sharedId = dim[0];
                const w = dim[1][0];
                const h = dim[1][1];

                this._shared_img_id = sharedId;
                this.width = w;
                this.height = h;

                this._loaded = true;
                this._error = null;

                this._onload && this._onload();
                this._fireListeners('load', undefined);
            })
            .catch((err) => {
                if (seq !== this._load_seq) return;

                this._shared_img_id = this._rid; // fall back
                this._loaded = false;
                this._error = err;

                this._onerror && this._onerror(err);
                this._fireListeners('error', err);
            });
    }
}

const createImage = () => new Image();

/**
 * ImagePreloader - Preload multiple images in parallel for better performance
 *
 * Usage:
 *   const preloader = new ImagePreloader();
 *   const results = await preloader.preload(["img1.png", "img2.png"]);
 *   // After preloading, images load instantly from cache
 *   const img = createImage();
 *   img.src = "img1.png"; // instant load from cache
 */
class ImagePreloader {
    constructor() {
        this._results = new Map();
    }

    /**
     * Preload multiple images in parallel
     * @param {string[]} paths - Array of image paths to preload
     * @returns {Promise<PreloadResult[]>} Array of results
     */
    async preload(paths) {
        if (!Array.isArray(paths) || paths.length === 0) {
            return [];
        }

        const results = await op_preload_images(paths);

        // Store results for later query
        const output = results.map(([path, success, width, height, errorMsg]) => {
            const result = {
                path,
                success,
                width: success ? width : 0,
                height: success ? height : 0,
                error: success ? null : errorMsg
            };
            this._results.set(path, result);
            return result;
        });

        return output;
    }

    /**
     * Check if a path was preloaded successfully
     * @param {string} path
     * @returns {boolean}
     */
    isLoaded(path) {
        const result = this._results.get(path);
        return result?.success ?? false;
    }

    /**
     * Get preload result for a path
     * @param {string} path
     * @returns {PreloadResult|null}
     */
    getResult(path) {
        return this._results.get(path) || null;
    }

    /**
     * Get all preload results
     * @returns {Map<string, PreloadResult>}
     */
    getAllResults() {
        return new Map(this._results);
    }

    /**
     * Clear preload results (does not clear the actual cache)
     */
    clearResults() {
        this._results.clear();
    }
}

/**
 * ImageCache - Utility for managing the image cache
 */
const ImageCache = {
    /**
     * Clear the entire image cache
     * Useful for memory management in resource-constrained environments
     */
    async clear() {
        await op_clear_image_cache();
    },

    /**
     * Get cache statistics
     * @returns {Promise<CacheStats>}
     */
    async getStats() {
        return await op_get_image_cache_stats();
    }
};

export { createImage, ImagePreloader, ImageCache };
