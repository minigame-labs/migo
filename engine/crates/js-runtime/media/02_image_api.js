import {
    op_save_image_to_photos_album,
    op_preview_media,
    op_preview_image,
    op_compress_image,
    op_choose_message_file,
    op_choose_image,
} from "ext:core/ops";
import { wrapAsync, createDeferredApi } from "ext:host_v8_base/02_async.js";


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

// ==================== compressImage (async, Mode C) ====================

const _compressImageApi = createDeferredApi('compressImage');

function compressImage(options = {}) {
    return _compressImageApi.invoke(options, function (opts) {
        if (!opts.src) {
            throw new Error('src is required');
        }
        op_compress_image(JSON.stringify({
            src: opts.src,
            quality: opts.quality !== undefined ? opts.quality : 80,
            compressedWidth: opts.compressedWidth !== undefined ? opts.compressedWidth : 0,
            compressedHeight: opts.compressedHeight !== undefined ? opts.compressedHeight : 0,
        }));
    });
}

function _internalOnCompressImageResult(resultJson) {
    _compressImageApi.settle(resultJson);
}

// ==================== chooseMessageFile (async, Mode C) ====================

const _chooseMessageFileApi = createDeferredApi('chooseMessageFile');

function chooseMessageFile(options = {}) {
    return _chooseMessageFileApi.invoke(options, function (opts) {
        if (opts.count === undefined) {
            throw new Error('count is required');
        }
        op_choose_message_file(JSON.stringify({
            count: opts.count,
            type: opts.type !== undefined ? opts.type : 'all',
            extension: opts.extension !== undefined ? opts.extension : [],
        }));
    });
}

function _internalOnChooseMessageFileResult(resultJson) {
    _chooseMessageFileApi.settle(resultJson);
}

// ==================== chooseImage (async, Mode C) ====================

const _chooseImageApi = createDeferredApi('chooseImage');

function chooseImage(options = {}) {
    return _chooseImageApi.invoke(options, function (opts) {
        op_choose_image(JSON.stringify({
            count: opts.count !== undefined ? opts.count : 9,
            sizeType: opts.sizeType !== undefined ? opts.sizeType : ['original', 'compressed'],
            sourceType: opts.sourceType !== undefined ? opts.sourceType : ['album', 'camera'],
        }));
    });
}

function _internalOnChooseImageResult(resultJson) {
    _chooseImageApi.settle(resultJson);
}

export {
    saveImageToPhotosAlbum,
    previewMedia,
    previewImage,
    compressImage,
    _internalOnCompressImageResult,
    chooseMessageFile,
    _internalOnChooseMessageFileResult,
    chooseImage,
    _internalOnChooseImageResult,
};
