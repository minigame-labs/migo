import {
    op_save_image_to_photos_album,
    op_preview_media,
    op_preview_image,
    op_compress_image,
    op_choose_message_file,
    op_choose_image,
} from "ext:core/ops";
import { wrapAsync } from "ext:host_v8_base/02_async.js";

const noop = () => {};

// ==================== saveImageToPhotosAlbum ====================

function saveImageToPhotosAlbum(options = {}) {
    const { filePath } = options;
    return wrapAsync('saveImageToPhotosAlbum', function () {
        if (!filePath) {
            throw new Error('filePath is required');
        }
        op_save_image_to_photos_album(JSON.stringify({ filePath }));
    }, options);
}

// ==================== previewMedia ====================

function previewMedia(options = {}) {
    const { sources, current, showmenu, referrerPolicy } = options;
    return wrapAsync('previewMedia', function () {
        if (!sources || !Array.isArray(sources) || sources.length === 0) {
            throw new Error('sources is required');
        }
        op_preview_media(JSON.stringify({
            sources,
            current: current !== undefined ? current : 0,
            showmenu: showmenu !== undefined ? showmenu : true,
            referrerPolicy: referrerPolicy !== undefined ? referrerPolicy : 'no-referrer',
        }));
    }, options);
}

// ==================== previewImage ====================

function previewImage(options = {}) {
    const { urls, current, showmenu, referrerPolicy } = options;
    return wrapAsync('previewImage', function () {
        if (!urls || !Array.isArray(urls) || urls.length === 0) {
            throw new Error('urls is required');
        }
        op_preview_image(JSON.stringify({
            urls,
            current: current !== undefined ? current : urls[0],
            showmenu: showmenu !== undefined ? showmenu : true,
            referrerPolicy: referrerPolicy !== undefined ? referrerPolicy : 'no-referrer',
        }));
    }, options);
}

// ==================== compressImage ====================

function compressImage(options = {}) {
    const { src, quality, compressedWidth, compressedHeight } = options;
    return wrapAsync('compressImage', function () {
        if (!src) {
            throw new Error('src is required');
        }
        const resultJson = op_compress_image(JSON.stringify({
            src,
            quality: quality !== undefined ? quality : 80,
            compressedWidth: compressedWidth !== undefined ? compressedWidth : 0,
            compressedHeight: compressedHeight !== undefined ? compressedHeight : 0,
        }));
        return JSON.parse(resultJson);
    }, options);
}

// ==================== chooseMessageFile (async) ====================

let _chooseMessageFileSuccess = null;
let _chooseMessageFileFail = null;
let _chooseMessageFileComplete = null;

function chooseMessageFile(options = {}) {
    const { count, type: fileType, extension, success, fail, complete } = options;

    _chooseMessageFileSuccess = success || noop;
    _chooseMessageFileFail = fail || noop;
    _chooseMessageFileComplete = complete || noop;

    try {
        if (count === undefined) {
            throw new Error('count is required');
        }
        op_choose_message_file(JSON.stringify({
            count,
            type: fileType !== undefined ? fileType : 'all',
            extension: extension !== undefined ? extension : [],
        }));
    } catch (e) {
        const res = { errMsg: 'chooseMessageFile:fail ' + e.message };
        _chooseMessageFileFail(res);
        _chooseMessageFileComplete(res);
        _chooseMessageFileSuccess = null;
        _chooseMessageFileFail = null;
        _chooseMessageFileComplete = null;
    }
}

function _internalOnChooseMessageFileResult(resultJson) {
    let parsed;
    try { parsed = JSON.parse(resultJson); } catch (e) { parsed = {}; }

    if (parsed.error) {
        const res = { errMsg: 'chooseMessageFile:fail ' + parsed.error };
        const f = _chooseMessageFileFail;
        const c = _chooseMessageFileComplete;
        _chooseMessageFileSuccess = null;
        _chooseMessageFileFail = null;
        _chooseMessageFileComplete = null;
        if (f) f(res);
        if (c) c(res);
    } else {
        const res = {
            tempFiles: parsed.tempFiles || [],
            errMsg: 'chooseMessageFile:ok',
        };
        const s = _chooseMessageFileSuccess;
        const c = _chooseMessageFileComplete;
        _chooseMessageFileSuccess = null;
        _chooseMessageFileFail = null;
        _chooseMessageFileComplete = null;
        if (s) s(res);
        if (c) c(res);
    }
}

// ==================== chooseImage (async) ====================

let _chooseImageSuccess = null;
let _chooseImageFail = null;
let _chooseImageComplete = null;

function chooseImage(options = {}) {
    const { count, sizeType, sourceType, success, fail, complete } = options;

    _chooseImageSuccess = success || noop;
    _chooseImageFail = fail || noop;
    _chooseImageComplete = complete || noop;

    try {
        op_choose_image(JSON.stringify({
            count: count !== undefined ? count : 9,
            sizeType: sizeType !== undefined ? sizeType : ['original', 'compressed'],
            sourceType: sourceType !== undefined ? sourceType : ['album', 'camera'],
        }));
    } catch (e) {
        const res = { errMsg: 'chooseImage:fail ' + e.message };
        _chooseImageFail(res);
        _chooseImageComplete(res);
        _chooseImageSuccess = null;
        _chooseImageFail = null;
        _chooseImageComplete = null;
    }
}

function _internalOnChooseImageResult(resultJson) {
    let parsed;
    try { parsed = JSON.parse(resultJson); } catch (e) { parsed = {}; }

    if (parsed.error) {
        const res = { errMsg: 'chooseImage:fail ' + parsed.error };
        const f = _chooseImageFail;
        const c = _chooseImageComplete;
        _chooseImageSuccess = null;
        _chooseImageFail = null;
        _chooseImageComplete = null;
        if (f) f(res);
        if (c) c(res);
    } else {
        const res = {
            tempFilePaths: parsed.tempFilePaths || [],
            tempFiles: parsed.tempFiles || [],
            errMsg: 'chooseImage:ok',
        };
        const s = _chooseImageSuccess;
        const c = _chooseImageComplete;
        _chooseImageSuccess = null;
        _chooseImageFail = null;
        _chooseImageComplete = null;
        if (s) s(res);
        if (c) c(res);
    }
}

export {
    saveImageToPhotosAlbum,
    previewMedia,
    previewImage,
    compressImage,
    chooseMessageFile,
    _internalOnChooseMessageFileResult,
    chooseImage,
    _internalOnChooseImageResult,
};
