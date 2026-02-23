
import {
  op_camera_create,
  op_camera_destroy,
  op_camera_take_photo,
  op_camera_start_record,
  op_camera_stop_record,
  op_camera_set_zoom,
  op_camera_listen_frame_change,
  op_camera_close_frame_change,
} from "ext:core/ops";

// Camera instance registry: cameraId -> Camera
const _cameras = new Map();

// Auto-incrementing camera ID counter
let _nextCameraId = 1;

/**
 * Camera - Provides access to device camera for photo capture,
 * video recording, and real-time frame streaming.
 *
 * Created via createCamera(). Each instance has a unique ID
 * used to route native events back to the correct instance.
 */
class Camera {
  #id;
  #destroyed = false;

  // Multi-listener arrays for event callbacks
  #listeners = {
    cameraFrame: [],
    stop: [],
    authCancel: [],
    error: [],
  };

  constructor(id) {
    this.#id = id;
  }

  // ==================== Frame Streaming ====================

  /**
   * Start listening for camera frame changes.
   * Begins high-frequency frame data streaming from the native camera.
   *
   * @param {Function} callback - Receives {data: ArrayBuffer, width: number, height: number}
   */
  listenFrameChange(callback) {
    if (typeof callback === "function") {
      this.#listeners.cameraFrame.push(callback);
    }
    try {
      op_camera_listen_frame_change(this.#id);
    } catch (e) {
      this.#fireListeners("error", { errMsg: e.message || String(e) });
    }
  }

  /**
   * Stop listening for camera frame changes.
   */
  closeFrameChange() {
    try {
      op_camera_close_frame_change(this.#id);
    } catch (e) {
      this.#fireListeners("error", { errMsg: e.message || String(e) });
    }
  }

  /**
   * Register a callback for camera frame data.
   * Unlike listenFrameChange, this only registers the callback
   * without starting native frame streaming.
   *
   * @param {Function} callback - Receives {data: ArrayBuffer, width: number, height: number}
   */
  onCameraFrame(callback) {
    if (typeof callback === "function") {
      this.#listeners.cameraFrame.push(callback);
    }
  }

  /**
   * Register a callback for camera stop events.
   * @param {Function} callback
   */
  onStop(callback) {
    if (typeof callback === "function") {
      this.#listeners.stop.push(callback);
    }
  }

  /**
   * Register a callback for authorization cancellation.
   * @param {Function} callback
   */
  onAuthCancel(callback) {
    if (typeof callback === "function") {
      this.#listeners.authCancel.push(callback);
    }
  }

  // ==================== Photo Capture ====================

  /**
   * Take a photo.
   *
   * @param {Object} [options={}]
   * @param {string} [options.quality="normal"] - "high", "normal", or "low"
   * @returns {Promise<{tempImagePath: string}>}
   */
  takePhoto(options = {}) {
    const opts = JSON.stringify({
      cameraId: this.#id,
      quality: options.quality ?? "normal",
    });

    return new Promise((resolve, reject) => {
      try {
        const result = JSON.parse(op_camera_take_photo(opts));
        resolve(result);
      } catch (e) {
        reject({ errMsg: "camera.takePhoto:fail " + (e.message || String(e)) });
      }
    });
  }

  // ==================== Video Recording ====================

  /**
   * Start video recording.
   *
   * @param {Object} [options={}]
   * @param {number} [options.timeout] - Max recording duration in seconds
   * @param {Function} [options.timeoutCallback] - Called when timeout is reached
   * @returns {Promise<void>}
   */
  startRecord(options = {}) {
    const opts = JSON.stringify({
      cameraId: this.#id,
      timeout: options.timeout,
    });

    // Store timeout callback for later dispatch
    if (typeof options.timeoutCallback === "function") {
      this._timeoutCallback = options.timeoutCallback;
    }

    return new Promise((resolve, reject) => {
      try {
        op_camera_start_record(opts);
        resolve();
      } catch (e) {
        reject({ errMsg: "camera.startRecord:fail " + (e.message || String(e)) });
      }
    });
  }

  /**
   * Stop video recording.
   *
   * @param {Object} [options={}]
   * @param {boolean} [options.compressed=false] - Whether to compress the video
   * @returns {Promise<{tempThumbPath: string, tempVideoPath: string}>}
   */
  stopRecord(options = {}) {
    const opts = JSON.stringify({
      cameraId: this.#id,
      compressed: options.compressed ?? false,
    });

    return new Promise((resolve, reject) => {
      try {
        const result = JSON.parse(op_camera_stop_record(opts));
        resolve(result);
      } catch (e) {
        reject({ errMsg: "camera.stopRecord:fail " + (e.message || String(e)) });
      }
    });
  }

  // ==================== Zoom ====================

  /**
   * Set camera zoom level.
   *
   * @param {Object} options
   * @param {number} options.zoom - Zoom factor to set
   * @returns {Promise<{zoom: number}>} Actual zoom level applied
   */
  setZoom(options = {}) {
    const opts = JSON.stringify({
      cameraId: this.#id,
      zoom: options.zoom ?? 1,
    });

    return new Promise((resolve, reject) => {
      try {
        const result = JSON.parse(op_camera_set_zoom(opts));
        resolve(result);
      } catch (e) {
        reject({ errMsg: "camera.setZoom:fail " + (e.message || String(e)) });
      }
    });
  }

  // ==================== Lifecycle ====================

  /**
   * Destroy the camera instance and release all resources.
   */
  destroy() {
    if (this.#destroyed) return;
    this.#destroyed = true;
    _cameras.delete(this.#id);

    try {
      op_camera_destroy(this.#id);
    } catch (e) {
      // Ignore errors during cleanup
    }

    // Clear all listeners
    for (const key of Object.keys(this.#listeners)) {
      this.#listeners[key].length = 0;
    }
  }

  // ==================== Internal ====================

  /** Fire all listeners for the given event type. */
  #fireListeners(type, arg) {
    const list = this.#listeners[type];
    if (!list || list.length === 0) return;
    const snapshot = list.slice();
    for (const fn of snapshot) {
      try {
        fn(arg);
      } catch (e) {
        console.error("Camera callback error:", e);
      }
    }
  }

  /**
   * Internal: handle event from native layer.
   * @param {string} eventType - "stop", "authCancel", "error", "timeoutCallback"
   * @param {string} jsonPayload - JSON data
   */
  _handleEvent(eventType, jsonPayload) {
    switch (eventType) {
      case "stop":
        this.#fireListeners("stop");
        break;
      case "authCancel":
        this.#fireListeners("authCancel");
        break;
      case "error": {
        let errObj = { errMsg: "Unknown error" };
        try {
          errObj = JSON.parse(jsonPayload);
        } catch (_) {}
        this.#fireListeners("error", errObj);
        break;
      }
      case "timeoutCallback":
        if (typeof this._timeoutCallback === "function") {
          try {
            this._timeoutCallback();
          } catch (e) {
            console.error("Camera timeoutCallback error:", e);
          }
        }
        break;
    }
  }

  /**
   * Internal: handle frame data from native layer.
   * @param {ArrayBuffer} data - Raw pixel data (RGBA)
   * @param {number} width - Frame width in pixels
   * @param {number} height - Frame height in pixels
   */
  _handleFrameData(data, width, height) {
    this.#fireListeners("cameraFrame", { data, width, height });
  }
}

/**
 * Create a Camera instance.
 *
 * @param {Object} [options={}]
 * @param {string} [options.pos="back"] - Camera position: "back" or "front"
 * @param {string} [options.flash="auto"] - Flash mode: "auto", "on", "off", "torch"
 * @param {string} [options.size="medium"] - Preview size: "small", "medium", "large"
 * @param {Function} [options.success] - Success callback
 * @param {Function} [options.fail] - Failure callback
 * @param {Function} [options.complete] - Complete callback (always called)
 * @returns {Camera}
 */
function createCamera(options = {}) {
  const cameraId = _nextCameraId++;
  const opts = JSON.stringify({
    cameraId,
    pos: options.pos ?? "back",
    flash: options.flash ?? "auto",
    size: options.size ?? "medium",
  });

  let camera;
  try {
    op_camera_create(opts);
    camera = new Camera(cameraId);
    _cameras.set(cameraId, camera);

    if (typeof options.success === "function") {
      options.success({ camera });
    }
  } catch (e) {
    const errMsg = "createCamera:fail " + (e.message || String(e));
    if (typeof options.fail === "function") {
      options.fail({ errMsg });
    }
    // Still create the camera object for consistency, but mark failure
    if (!camera) {
      camera = new Camera(cameraId);
      _cameras.set(cameraId, camera);
    }
  } finally {
    if (typeof options.complete === "function") {
      options.complete();
    }
  }

  return camera;
}

/**
 * Internal: called from native to dispatch camera events.
 * @param {number} cameraId - Camera instance ID
 * @param {string} eventType - Event type
 * @param {string} jsonPayload - JSON payload
 */
function _internalOnCameraEvent(cameraId, eventType, jsonPayload) {
  const camera = _cameras.get(cameraId);
  if (camera) {
    camera._handleEvent(eventType, jsonPayload);
  }
}

/**
 * Internal: called from native to dispatch camera frame data.
 * @param {number} cameraId - Camera instance ID
 * @param {ArrayBuffer} data - Raw pixel data
 * @param {number} width - Frame width
 * @param {number} height - Frame height
 */
function _internalOnCameraFrameData(cameraId, data, width, height) {
  const camera = _cameras.get(cameraId);
  if (camera) {
    camera._handleFrameData(data, width, height);
  }
}

export {
  Camera,
  createCamera,
  _internalOnCameraEvent,
  _internalOnCameraFrameData,
};
