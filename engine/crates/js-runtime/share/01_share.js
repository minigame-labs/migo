// showShareMenu / updateShareMenu / onShareAppMessage /
// offShareAppMessage / shareAppMessage
//
// Minimal viable implementation:
//   - showShareMenu / updateShareMenu cache config, return :ok
//   - onShareAppMessage registers a callback invoked when shareAppMessage fires
//   - shareAppMessage delegates to the host via _internalOnShareRequest if present

import { op_share_app_message } from "ext:core/ops";
import { wrapAsync, createDeferredApi } from "ext:host_v8_base/02_async.js";

// ---- share menu state ------------------------------------------------------

let _shareMenuConfig = {
    withShareTicket: false,
    menus: ['shareAppMessage', 'shareTimeline'],
};

// ---- showShareMenu ---------------------------------------------------------

function showShareMenu(options) {
    return wrapAsync('showShareMenu', function () {
        const opts = options || {};
        if (opts.withShareTicket !== undefined) {
            _shareMenuConfig.withShareTicket = !!opts.withShareTicket;
        }
        if (Array.isArray(opts.menus)) {
            _shareMenuConfig.menus = opts.menus.slice();
        }
        return {};
    }, options);
}

// ---- hideShareMenu ---------------------------------------------------------

function hideShareMenu(options) {
    return wrapAsync('hideShareMenu', function () {
        _shareMenuConfig.menus = [];
        return {};
    }, options);
}

// ---- updateShareMenu -------------------------------------------------------

function updateShareMenu(options) {
    return wrapAsync('updateShareMenu', function () {
        const opts = options || {};
        if (opts.withShareTicket !== undefined) {
            _shareMenuConfig.withShareTicket = !!opts.withShareTicket;
        }
        if (opts.isUpdatableMessage !== undefined) {
            _shareMenuConfig.isUpdatableMessage = !!opts.isUpdatableMessage;
        }
        if (opts.activityId !== undefined) {
            _shareMenuConfig.activityId = opts.activityId;
        }
        if (opts.templateInfo !== undefined) {
            _shareMenuConfig.templateInfo = opts.templateInfo;
        }
        if (Array.isArray(opts.menus)) {
            _shareMenuConfig.menus = opts.menus.slice();
        }
        return {};
    }, options);
}

// ---- onShareAppMessage / offShareAppMessage --------------------------------

const _shareListeners = [];

function onShareAppMessage(listener) {
    if (typeof listener === 'function') {
        _shareListeners.push(listener);
    }
}

function offShareAppMessage(listener) {
    if (typeof listener === 'function') {
        const index = _shareListeners.indexOf(listener);
        if (index !== -1) {
            _shareListeners.splice(index, 1);
        }
    } else {
        _shareListeners.length = 0;
    }
}

// ---- shareAppMessage (Mode C - host op) ------------------------------------

const _shareAppMessageApi = createDeferredApi('shareAppMessage');

function shareAppMessage(options) {
    return _shareAppMessageApi.invoke(options, function (opts, requestId) {
        const shareData = {
            requestId: requestId,
            title: opts.title || '',
            imageUrl: opts.imageUrl || '',
            query: opts.query || '',
            imageUrlId: opts.imageUrlId || '',
        };

        // Invoke registered listeners to allow the game to customise share data
        for (let i = 0; i < _shareListeners.length; i++) {
            try {
                const override = _shareListeners[i](shareData);
                if (override && typeof override === 'object') {
                    if (typeof override.title === 'string') shareData.title = override.title;
                    if (typeof override.imageUrl === 'string') shareData.imageUrl = override.imageUrl;
                    if (typeof override.query === 'string') shareData.query = override.query;
                }
            } catch (e) {
                console.error('onShareAppMessage listener error:', e);
            }
        }

        op_share_app_message(JSON.stringify(shareData));
    });
}

function _internalOnShareAppMessageResult(resultJson) {
    _shareAppMessageApi.settle(resultJson);
}

// ---- onShareTimeline / offShareTimeline ------------------------------------

const _shareTimelineListeners = [];

function onShareTimeline(listener) {
    if (typeof listener === 'function') {
        _shareTimelineListeners.push(listener);
    }
}

function offShareTimeline(listener) {
    if (typeof listener === 'function') {
        const index = _shareTimelineListeners.indexOf(listener);
        if (index !== -1) _shareTimelineListeners.splice(index, 1);
    } else {
        _shareTimelineListeners.length = 0;
    }
}

// ---- host-side trigger (called from Rust when user taps native timeline share)

function _internalTriggerShareTimeline() {
    var shareData = { title: '', imageUrl: '', query: '' };
    for (var i = 0; i < _shareTimelineListeners.length; i++) {
        try {
            var override = _shareTimelineListeners[i]();
            if (override && typeof override === 'object') {
                if (typeof override.title === 'string') shareData.title = override.title;
                if (typeof override.imageUrl === 'string') shareData.imageUrl = override.imageUrl;
                if (typeof override.query === 'string') shareData.query = override.query;
            }
        } catch (e) {
            console.error('onShareTimeline listener error:', e);
        }
    }
    return shareData;
}

// ---- shareMessageToFriend (Mode A stub) ------------------------------------

function shareMessageToFriend(options) {
    return wrapAsync('shareMessageToFriend', function () {
        var opts = options || {};
        if (!opts.openId) throw new Error('openId is required');
        op_share_app_message(JSON.stringify({
            type: 'shareMessageToFriend',
            openId: opts.openId,
            title: opts.title || '',
            imageUrl: opts.imageUrl || '',
            imageUrlId: opts.imageUrlId || '',
        }));
    }, options);
}

// ---- onShareMessageToFriend / offShareMessageToFriend ----------------------

const _shareToFriendListeners = [];

function onShareMessageToFriend(listener) {
    if (typeof listener === 'function') {
        _shareToFriendListeners.push(listener);
    }
}

function offShareMessageToFriend(listener) {
    if (typeof listener === 'function') {
        const index = _shareToFriendListeners.indexOf(listener);
        if (index !== -1) _shareToFriendListeners.splice(index, 1);
    } else {
        _shareToFriendListeners.length = 0;
    }
}

function _internalTriggerShareMessageToFriend(data) {
    var parsed = data;
    if (typeof data === 'string') {
        try { parsed = JSON.parse(data); } catch (_) { parsed = {}; }
    }
    for (let i = 0; i < _shareToFriendListeners.length; i++) {
        try { _shareToFriendListeners[i](parsed); } catch (e) {
            console.error('onShareMessageToFriend listener error:', e);
        }
    }
}

// ---- setMessageToFriendQuery -----------------------------------------------

let _messageToFriendQuery = '';

function setMessageToFriendQuery(options) {
    var opts = options || {};
    if (typeof opts.query === 'string') {
        _messageToFriendQuery = opts.query;
    }
}

// ---- showShareImageMenu (Mode A stub) --------------------------------------

function showShareImageMenu(options) {
    return wrapAsync('showShareImageMenu', function () {
        var opts = options || {};
        op_share_app_message(JSON.stringify({
            type: 'showShareImageMenu',
            path: opts.path || '',
            imageUrl: opts.imageUrl || '',
            style: opts.style || '',
            needShowEntrance: !!opts.needShowEntrance,
            entrancePath: opts.entrancePath || '',
        }));
    }, options);
}

// ---- host-side trigger (called from Rust when user taps native share) ------

function _internalTriggerShareAppMessage() {
    let shareData = { title: '', imageUrl: '', query: '' };
    for (let i = 0; i < _shareListeners.length; i++) {
        try {
            const override = _shareListeners[i]();
            if (override && typeof override === 'object') {
                if (typeof override.title === 'string') shareData.title = override.title;
                if (typeof override.imageUrl === 'string') shareData.imageUrl = override.imageUrl;
                if (typeof override.query === 'string') shareData.query = override.query;
            }
        } catch (e) {
            console.error('onShareAppMessage listener error:', e);
        }
    }
    return shareData;
}

export {
    showShareMenu,
    hideShareMenu,
    updateShareMenu,
    onShareAppMessage,
    offShareAppMessage,
    shareAppMessage,
    _internalOnShareAppMessageResult,
    _internalTriggerShareAppMessage,
    onShareTimeline,
    offShareTimeline,
    _internalTriggerShareTimeline,
    shareMessageToFriend,
    onShareMessageToFriend,
    offShareMessageToFriend,
    _internalTriggerShareMessageToFriend,
    setMessageToFriendQuery,
    showShareImageMenu,
};
