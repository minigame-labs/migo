import { primordials } from "ext:core/mod.js";
import { op_create_image, op_load_image, op_destroy_image } from "ext:core/ops";
import { HTMLElement } from "ext:host_v8_web/02_html_element.js";

const { SafeFinalizationRegistry } = primordials;

const registry = new SafeFinalizationRegistry((rid) => {
    try {
        op_destroy_image(rid);
    } catch (_) { }
});

class Image extends HTMLElement {
    constructor() {
        super("Image");

        this._src = "";
        this.width = 0;
        this.height = 0;

        this._onload = null;
        this._onerror = null;

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
            })
            .catch((err) => {
                if (seq !== this._load_seq) return;

                this._shared_img_id = this._rid; // fall back
                this._loaded = false;
                this._error = err;

                this._onerror && this._onerror(err);
            });
    }
}

const createImage = () => new Image();

export { createImage };
