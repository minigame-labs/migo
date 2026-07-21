import { wrapAsync } from "ext:host_v8_base/02_async.js";

// Cached appId -- set by host via _internalSetAppId(id) at startup.
var _appId = "";

// Compatibility state for lightweight platform-only capabilities.
var _colorSignExists = false;
var _recentColorSignQuery = '';
var _secondFloorState = {
    showEnable: false,
    isSubscribe: false,
    channelId: '',
    pageCid: '',
    status: '',
};

function _internalSetAppId(appId) {
    if (typeof appId === "string") _appId = appId;
}

function getAppBaseInfo() {
    const appInfo = {
        SDKVersion: "4.0.0",
        enableDebug: false,
        host: {
            appId: "com.minigame.host",
        },
        language: "zh_CN",
        version: "1.0.0",
        theme: "light",
        fontSizeScaleFactor: 1.0,
        fontSizeSetting: 16
    };

    return appInfo;
}

function getAccountInfoSync() {
    return {
        miniProgram: {
            appId: _appId,
            envVersion: "release",
            version: "1.0.0",
        },
        plugin: {},
    };
}

function checkIsAddedToMyMiniProgram(options) {
    return wrapAsync('checkIsAddedToMyMiniProgram', function () {
        return { added: false };
    }, options);
}

function isColorSignExistSync() {
    return _colorSignExists;
}

function addColorSign(options) {
    return wrapAsync('addColorSign', function () {
        _colorSignExists = true;
        return {};
    }, options);
}

function addRecentColorSign(options) {
    return wrapAsync('addRecentColorSign', function () {
        var opts = options || {};
        if (typeof opts.query === 'string') {
            _recentColorSignQuery = opts.query;
        }
        return { query: _recentColorSignQuery };
    }, options);
}

function fetchSecondFloorIconOptionSync(options) {
    var opts = options || {};
    if (typeof opts.channelId === 'string') {
        _secondFloorState.channelId = opts.channelId;
    }
    return {
        showEnable: !!_secondFloorState.showEnable,
        isSubscribe: !!_secondFloorState.isSubscribe,
    };
}

function updateSecondFloorChannel(options) {
    return wrapAsync('updateSecondFloorChannel', function () {
        var opts = options || {};
        if (typeof opts.channelId === 'string') {
            _secondFloorState.channelId = opts.channelId;
        }
        if (typeof opts.pageCid === 'string') {
            _secondFloorState.pageCid = opts.pageCid;
        }
        if (typeof opts.status === 'string') {
            _secondFloorState.status = opts.status;
            _secondFloorState.showEnable = opts.status !== 'close';
        }
        return {
            showEnable: !!_secondFloorState.showEnable,
            isSubscribe: !!_secondFloorState.isSubscribe,
        };
    }, options);
}

export {
    getAppBaseInfo,
    getAccountInfoSync,
    checkIsAddedToMyMiniProgram,
    isColorSignExistSync,
    addColorSign,
    addRecentColorSign,
    fetchSecondFloorIconOptionSync,
    updateSecondFloorChannel,
    _internalSetAppId,
};
