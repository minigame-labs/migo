import {
    op_video_create,
    op_video_play,
    op_video_pause,
    op_video_stop,
    op_video_seek,
    op_video_request_fullscreen,
    op_video_exit_fullscreen,
    op_video_set_property,
    op_video_destroy,
} from "ext:core/ops";
import { wrapAsync } from "ext:host_v8_base/02_async.js";

// Video instance registry: videoId -> Video
var _videos = new Map();

// Auto-incrementing video ID counter
var _nextVideoId = 1;

/**
 * Video - Provides video playback capability.
 *
 * Created via createVideo(). Each instance has a unique ID
 * used to route native events back to the correct instance.
 */
class Video {
    #id;
    #destroyed = false;

    // Playback state (updated by native events)
    #src = '';
    #duration = 0;
    #currentTime = 0;
    #paused = true;
    #loop = false;
    #muted = false;
    #playbackRate = 1.0;
    #objectFit = 'contain';

    // Multi-listener arrays for event callbacks
    #listeners = {
        play: [],
        pause: [],
        ended: [],
        timeupdate: [],
        error: [],
        waiting: [],
        progress: [],
        fullscreenchange: [],
    };

    constructor(id, options) {
        this.#id = id;
        if (options.src !== undefined) this.#src = String(options.src);
        if (options.loop !== undefined) this.#loop = !!options.loop;
        if (options.muted !== undefined) this.#muted = !!options.muted;
        if (options.playbackRate !== undefined) this.#playbackRate = Number(options.playbackRate) || 1.0;
        if (options.objectFit !== undefined) this.#objectFit = String(options.objectFit);
    }

    // ==================== Properties ====================

    get src() { return this.#src; }
    set src(val) {
        this.#src = String(val);
        this.#syncProperty('src', this.#src);
    }

    get duration() { return this.#duration; }
    get currentTime() { return this.#currentTime; }
    get paused() { return this.#paused; }

    get loop() { return this.#loop; }
    set loop(val) {
        this.#loop = !!val;
        this.#syncProperty('loop', this.#loop);
    }

    get muted() { return this.#muted; }
    set muted(val) {
        this.#muted = !!val;
        this.#syncProperty('muted', this.#muted);
    }

    get playbackRate() { return this.#playbackRate; }
    set playbackRate(val) {
        this.#playbackRate = Number(val) || 1.0;
        this.#syncProperty('playbackRate', this.#playbackRate);
    }

    get objectFit() { return this.#objectFit; }
    set objectFit(val) {
        this.#objectFit = String(val);
        this.#syncProperty('objectFit', this.#objectFit);
    }

    // ==================== Playback Control ====================

    /**
     * Start or resume playback.
     * @param {Object} [options] - success/fail/complete callbacks
     * @returns {Promise}
     */
    play(options) {
        var self = this;
        return wrapAsync('video.play', function () {
            op_video_play(self.#id);
        }, options);
    }

    /**
     * Pause playback.
     * @param {Object} [options] - success/fail/complete callbacks
     * @returns {Promise}
     */
    pause(options) {
        var self = this;
        return wrapAsync('video.pause', function () {
            op_video_pause(self.#id);
        }, options);
    }

    /**
     * Stop playback and reset to beginning.
     * @param {Object} [options] - success/fail/complete callbacks
     * @returns {Promise}
     */
    stop(options) {
        var self = this;
        return wrapAsync('video.stop', function () {
            op_video_stop(self.#id);
        }, options);
    }

    /**
     * Seek to a given position.
     * @param {number} time - Position in seconds
     * @param {Object} [options] - success/fail/complete callbacks
     * @returns {Promise}
     */
    seek(time, options) {
        var self = this;
        var pos = Number(time) || 0;
        return wrapAsync('video.seek', function () {
            op_video_seek(self.#id, pos);
        }, options);
    }

    /**
     * Enter fullscreen mode.
     * @param {Object} [options]
     * @param {number} [options.direction=0] - 0: normal, 90: rotate 90, -90: rotate -90
     * @returns {Promise}
     */
    requestFullScreen(options) {
        var self = this;
        var opts = options || {};
        var direction = (opts.direction !== undefined) ? Number(opts.direction) : 0;
        return wrapAsync('video.requestFullScreen', function () {
            op_video_request_fullscreen(self.#id, direction);
        }, opts);
    }

    /**
     * Exit fullscreen mode.
     * @param {Object} [options] - success/fail/complete callbacks
     * @returns {Promise}
     */
    exitFullScreen(options) {
        var self = this;
        return wrapAsync('video.exitFullScreen', function () {
            op_video_exit_fullscreen(self.#id);
        }, options);
    }

    // ==================== Event Listeners ====================

    onPlay(callback) {
        if (typeof callback === 'function') this.#listeners.play.push(callback);
    }
    offPlay(callback) {
        this.#removeListener('play', callback);
    }

    onPause(callback) {
        if (typeof callback === 'function') this.#listeners.pause.push(callback);
    }
    offPause(callback) {
        this.#removeListener('pause', callback);
    }

    onEnded(callback) {
        if (typeof callback === 'function') this.#listeners.ended.push(callback);
    }
    offEnded(callback) {
        this.#removeListener('ended', callback);
    }

    onTimeUpdate(callback) {
        if (typeof callback === 'function') this.#listeners.timeupdate.push(callback);
    }
    offTimeUpdate(callback) {
        this.#removeListener('timeupdate', callback);
    }

    onError(callback) {
        if (typeof callback === 'function') this.#listeners.error.push(callback);
    }
    offError(callback) {
        this.#removeListener('error', callback);
    }

    onWaiting(callback) {
        if (typeof callback === 'function') this.#listeners.waiting.push(callback);
    }
    offWaiting(callback) {
        this.#removeListener('waiting', callback);
    }

    onProgress(callback) {
        if (typeof callback === 'function') this.#listeners.progress.push(callback);
    }
    offProgress(callback) {
        this.#removeListener('progress', callback);
    }

    onFullScreenChange(callback) {
        if (typeof callback === 'function') this.#listeners.fullscreenchange.push(callback);
    }
    offFullScreenChange(callback) {
        this.#removeListener('fullscreenchange', callback);
    }

    // ==================== Lifecycle ====================

    /**
     * Destroy the video player and release all resources.
     */
    destroy() {
        if (this.#destroyed) return;
        this.#destroyed = true;
        _videos.delete(this.#id);

        try {
            op_video_destroy(this.#id);
        } catch (e) {
            // Ignore errors during cleanup
        }

        // Clear all listeners
        for (var key of Object.keys(this.#listeners)) {
            this.#listeners[key].length = 0;
        }
    }

    // ==================== Internal ====================

    /** Remove a listener for the given event type. */
    #removeListener(type, callback) {
        var list = this.#listeners[type];
        if (!list) return;
        if (typeof callback === 'function') {
            var i = list.indexOf(callback);
            if (i !== -1) list.splice(i, 1);
        } else {
            list.length = 0;
        }
    }

    /** Fire all listeners for the given event type. */
    #fireListeners(type, arg) {
        var list = this.#listeners[type];
        if (!list || list.length === 0) return;
        var snapshot = list.slice();
        for (var i = 0; i < snapshot.length; i++) {
            try {
                snapshot[i](arg);
            } catch (e) {
                console.error('Video callback error:', e);
            }
        }
    }

    /** Sync a single property to native. */
    #syncProperty(name, value) {
        if (this.#destroyed) return;
        try {
            var obj = {};
            obj[name] = value;
            op_video_set_property(this.#id, JSON.stringify(obj));
        } catch (e) {
            // Silently ignore sync errors
        }
    }

    /**
     * Internal: handle event from native layer.
     * @param {string} eventType - Event type string
     * @param {string} dataJson - JSON payload
     */
    _handleEvent(eventType, dataJson) {
        var data = {};
        if (dataJson) {
            try { data = JSON.parse(dataJson); } catch (e) { /* empty */ }
        }

        switch (eventType) {
            case 'play':
                this.#paused = false;
                this.#fireListeners('play', data);
                break;
            case 'pause':
                this.#paused = true;
                this.#fireListeners('pause', data);
                break;
            case 'ended':
                this.#paused = true;
                this.#fireListeners('ended', data);
                break;
            case 'timeupdate':
                if (data.currentTime !== undefined) {
                    this.#currentTime = Number(data.currentTime);
                }
                if (data.duration !== undefined) {
                    this.#duration = Number(data.duration);
                }
                this.#fireListeners('timeupdate', {
                    position: this.#currentTime,
                    duration: this.#duration,
                });
                break;
            case 'waiting':
                this.#fireListeners('waiting', data);
                break;
            case 'progress':
                if (data.buffered !== undefined) {
                    this.#fireListeners('progress', { buffered: Number(data.buffered) });
                } else {
                    this.#fireListeners('progress', data);
                }
                break;
            case 'error':
                this.#fireListeners('error', data);
                break;
            case 'fullscreenchange':
                this.#fireListeners('fullscreenchange', data);
                break;
            default:
                break;
        }
    }
}

/**
 * Create a Video instance.
 *
 * @param {Object} [options={}]
 * @param {string} [options.src] - Video source URL
 * @param {number} [options.x=0] - Left position
 * @param {number} [options.y=0] - Top position
 * @param {number} [options.width=300] - Width
 * @param {number} [options.height=150] - Height
 * @param {boolean} [options.autoplay=false] - Auto-play on create
 * @param {boolean} [options.loop=false] - Loop playback
 * @param {boolean} [options.muted=false] - Muted
 * @param {number} [options.playbackRate=1.0] - Playback rate
 * @param {string} [options.objectFit='contain'] - Object fit mode
 * @param {string} [options.poster] - Poster image URL
 * @param {boolean} [options.controls=true] - Show native controls
 * @param {boolean} [options.showCenterPlayBtn=true] - Show center play button
 * @param {boolean} [options.enableProgressGesture=true] - Enable progress gesture
 * @returns {Video}
 */
function createVideo(options) {
    var opts = options || {};
    var videoId = _nextVideoId++;
    var createJson = JSON.stringify({
        videoId: videoId,
        src: opts.src !== undefined ? String(opts.src) : '',
        x: opts.x !== undefined ? Number(opts.x) : 0,
        y: opts.y !== undefined ? Number(opts.y) : 0,
        width: opts.width !== undefined ? Number(opts.width) : 300,
        height: opts.height !== undefined ? Number(opts.height) : 150,
        autoplay: !!opts.autoplay,
        loop: !!opts.loop,
        muted: !!opts.muted,
        playbackRate: (opts.playbackRate !== undefined) ? Number(opts.playbackRate) : 1.0,
        objectFit: opts.objectFit !== undefined ? String(opts.objectFit) : 'contain',
        poster: opts.poster !== undefined ? String(opts.poster) : '',
        controls: (opts.controls !== undefined) ? !!opts.controls : true,
        showCenterPlayBtn: (opts.showCenterPlayBtn !== undefined) ? !!opts.showCenterPlayBtn : true,
        enableProgressGesture: (opts.enableProgressGesture !== undefined) ? !!opts.enableProgressGesture : true,
    });

    try {
        op_video_create(createJson);
    } catch (e) {
        console.error('createVideo failed:', e);
    }

    var video = new Video(videoId, opts);
    _videos.set(videoId, video);
    return video;
}

/**
 * Internal: called from JsBindings to dispatch video events to the correct
 * Video instance.
 *
 * @param {number} videoId - Video instance ID
 * @param {string} eventType - Event type string
 * @param {string} dataJson - JSON-encoded event data
 */
function _internalTriggerVideoEvent(videoId, eventType, dataJson) {
    var video = _videos.get(videoId);
    if (video) {
        video._handleEvent(eventType, dataJson);
    }
}

function _emitSnapshot(listeners, payload) {
    if (!listeners || listeners.length === 0) return;
    var snapshot = listeners.slice();
    for (var i = 0; i < snapshot.length; i++) {
        try {
            snapshot[i](payload);
        } catch (e) {
            console.error('Live callback error:', e);
        }
    }
}

function _finishWithCallbacks(apiName, options, successPayload) {
    var payload = successPayload || {};
    payload.errMsg = apiName + ':ok';
    if (options && typeof options.success === 'function') {
        try { options.success(payload); } catch (e) {
            console.error(apiName + ' success callback error:', e);
        }
    }
    if (options && typeof options.complete === 'function') {
        try { options.complete(payload); } catch (e) {
            console.error(apiName + ' complete callback error:', e);
        }
    }
    return Promise.resolve(payload);
}

class LivePlayerContext {
    #destroyed = false;
    #playing = false;
    #muted = false;
    #volumeTimer = null;
    #listeners = {
        statechange: [],
        audiovolume: [],
    };

    constructor(options) {
        if (options && typeof options === 'object') {
            Object.assign(this, options);
        }
    }

    onStateChange(listener) {
        if (typeof listener === 'function') this.#listeners.statechange.push(listener);
    }

    offStateChange(listener) {
        if (typeof listener === 'function') {
            var i = this.#listeners.statechange.indexOf(listener);
            if (i !== -1) this.#listeners.statechange.splice(i, 1);
        } else {
            this.#listeners.statechange.length = 0;
        }
    }

    onAudioVolumeNotify(listener) {
        if (typeof listener === 'function') this.#listeners.audiovolume.push(listener);
    }

    offAudioVolumeNotify(listener) {
        if (typeof listener === 'function') {
            var i = this.#listeners.audiovolume.indexOf(listener);
            if (i !== -1) this.#listeners.audiovolume.splice(i, 1);
        } else {
            this.#listeners.audiovolume.length = 0;
        }
    }

    play(options) {
        if (this.#destroyed) return Promise.reject(new Error('livePlayer.play:fail destroyed'));
        this.#playing = true;
        this.#startVolumeTicker();
        _emitSnapshot(this.#listeners.statechange, { code: 2004, message: 'play' });
        return _finishWithCallbacks('livePlayer.play', options, {});
    }

    stop(options) {
        if (this.#destroyed) return Promise.reject(new Error('livePlayer.stop:fail destroyed'));
        this.#playing = false;
        this.#stopVolumeTicker();
        _emitSnapshot(this.#listeners.statechange, { code: 2006, message: 'stop' });
        return _finishWithCallbacks('livePlayer.stop', options, {});
    }

    pause(options) {
        if (this.#destroyed) return Promise.reject(new Error('livePlayer.pause:fail destroyed'));
        this.#playing = false;
        this.#stopVolumeTicker();
        _emitSnapshot(this.#listeners.statechange, { code: 2007, message: 'pause' });
        return _finishWithCallbacks('livePlayer.pause', options, {});
    }

    resume(options) {
        return this.play(options);
    }

    mute(options) {
        if (this.#destroyed) return Promise.reject(new Error('livePlayer.mute:fail destroyed'));
        this.#muted = !this.#muted;
        _emitSnapshot(this.#listeners.audiovolume, { volume: this.#muted ? 0 : 100 });
        return _finishWithCallbacks('livePlayer.mute', options, {});
    }

    destroy() {
        this.#destroyed = true;
        this.#playing = false;
        this.#stopVolumeTicker();
        this.#listeners.statechange.length = 0;
        this.#listeners.audiovolume.length = 0;
        return Promise.resolve();
    }

    #startVolumeTicker() {
        this.#stopVolumeTicker();
        this.#volumeTimer = setInterval(() => {
            if (!this.#playing || this.#destroyed) return;
            _emitSnapshot(this.#listeners.audiovolume, { volume: this.#muted ? 0 : 100 });
        }, 300);
    }

    #stopVolumeTicker() {
        if (this.#volumeTimer) {
            clearInterval(this.#volumeTimer);
            this.#volumeTimer = null;
        }
    }
}

class LivePusherContext {
    #destroyed = false;
    #started = false;
    #bgmTimer = null;
    #bgmProgress = 0;
    #listeners = {
        statechange: [],
        error: [],
        netstatus: [],
        bgmstart: [],
        bgmcomplete: [],
        bgmprogress: [],
    };

    constructor(options) {
        if (options && typeof options === 'object') {
            Object.assign(this, options);
        }
    }

    onStateChange(listener) {
        if (typeof listener === 'function') this.#listeners.statechange.push(listener);
    }

    onError(listener) {
        if (typeof listener === 'function') this.#listeners.error.push(listener);
    }

    onNetStatus(listener) {
        if (typeof listener === 'function') this.#listeners.netstatus.push(listener);
    }

    onBGMStart(listener) {
        if (typeof listener === 'function') this.#listeners.bgmstart.push(listener);
    }

    onBGMComplete(listener) {
        if (typeof listener === 'function') this.#listeners.bgmcomplete.push(listener);
    }

    onBGMProgress(listener) {
        if (typeof listener === 'function') this.#listeners.bgmprogress.push(listener);
    }

    start(options) {
        if (this.#destroyed) return Promise.reject(new Error('livePusher.start:fail destroyed'));
        this.#started = true;
        _emitSnapshot(this.#listeners.statechange, { code: 1001, message: 'start' });
        return _finishWithCallbacks('livePusher.start', options, {});
    }

    stop(options) {
        if (this.#destroyed) return Promise.reject(new Error('livePusher.stop:fail destroyed'));
        this.#started = false;
        _emitSnapshot(this.#listeners.statechange, { code: 1006, message: 'stop' });
        return _finishWithCallbacks('livePusher.stop', options, {});
    }

    pause(options) {
        if (this.#destroyed) return Promise.reject(new Error('livePusher.pause:fail destroyed'));
        _emitSnapshot(this.#listeners.statechange, { code: 1007, message: 'pause' });
        return _finishWithCallbacks('livePusher.pause', options, {});
    }

    resume(options) {
        if (this.#destroyed) return Promise.reject(new Error('livePusher.resume:fail destroyed'));
        _emitSnapshot(this.#listeners.statechange, { code: 1008, message: 'resume' });
        return _finishWithCallbacks('livePusher.resume', options, {});
    }

    playBGM(url) {
        if (this.#destroyed) return;
        this.#bgmProgress = 0;
        _emitSnapshot(this.#listeners.bgmstart, { url: typeof url === 'string' ? url : '' });
        this.#startBgmTicker();
    }

    pauseBGM() {
        if (this.#destroyed) return;
        this.#stopBgmTicker();
    }

    resumeBGM() {
        if (this.#destroyed) return;
        this.#startBgmTicker();
    }

    stopBGM() {
        if (this.#destroyed) return;
        this.#stopBgmTicker();
        _emitSnapshot(this.#listeners.bgmcomplete, { reason: 'stop' });
    }

    setBGMVolume(_volume) {}

    setMICVolume(_volume) {}

    destroy() {
        this.#destroyed = true;
        this.#started = false;
        this.#stopBgmTicker();
        this.#listeners.statechange.length = 0;
        this.#listeners.error.length = 0;
        this.#listeners.netstatus.length = 0;
        this.#listeners.bgmstart.length = 0;
        this.#listeners.bgmcomplete.length = 0;
        this.#listeners.bgmprogress.length = 0;
        return Promise.resolve();
    }

    #startBgmTicker() {
        this.#stopBgmTicker();
        this.#bgmTimer = setInterval(() => {
            if (this.#destroyed) return;
            this.#bgmProgress += 1000;
            _emitSnapshot(this.#listeners.bgmprogress, { progress: this.#bgmProgress });
            if (this.#bgmProgress >= 3000) {
                this.#stopBgmTicker();
                _emitSnapshot(this.#listeners.bgmcomplete, { reason: 'complete' });
            }
        }, 500);
    }

    #stopBgmTicker() {
        if (this.#bgmTimer) {
            clearInterval(this.#bgmTimer);
            this.#bgmTimer = null;
        }
    }
}

function createLivePlayer(options) {
    return new LivePlayerContext(options || {});
}

function createLivePusher(options) {
    return new LivePusherContext(options || {});
}

export {
    Video,
    createVideo,
    _internalTriggerVideoEvent,
    createLivePlayer,
    createLivePusher,
};
