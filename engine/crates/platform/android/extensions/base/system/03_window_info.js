
import { op_get_window_info } from "ext:core/ops";

class SafeArea {
    constructor(left, top, right, bottom) {
        this.left = left;
        this.top = top;
        this.right = right;
        this.bottom = bottom;
        this.width = right - left;
        this.height = bottom - top;
    }

    toJSON() {
        return {
            left: this.left,
            top: this.top,
            right: this.right,
            bottom: this.bottom,
            width: this.width,
            height: this.height
        };
    }
}

class WindowInfo {
    constructor(pixelRatio, screenWidth, screenHeight, windowWidth, windowHeight, statusBarHeight, screenTop, safeArea) {
        this.pixelRatio = pixelRatio;
        this.screenWidth = screenWidth;
        this.screenHeight = screenHeight;
        this.windowWidth = windowWidth;
        this.windowHeight = windowHeight;
        this.statusBarHeight = statusBarHeight;
        this.screenTop = screenTop;
        this.safeArea = safeArea;
    }

    toJSON() {
        return {
            pixelRatio: this.pixelRatio,
            screenWidth: this.screenWidth,
            screenHeight: this.screenHeight,
            windowWidth: this.windowWidth,
            windowHeight: this.windowHeight,
            statusBarHeight: this.statusBarHeight,
            screenTop: this.screenTop,
            safeArea: this.safeArea
        };
    }
}

function getWindowInfo() {
    const info = op_get_window_info()

    return new WindowInfo(
        info.pixel_ratio,
        info.screen_width,
        info.screen_height,
        info.window_width,
        info.window_height,
        info.status_bar_height,
        info.screen_top,
        new SafeArea(
            info.safe_area.left,
            info.safe_area.top,
            info.safe_area.right,
            info.safe_area.bottom
        )
    );
}

export { getWindowInfo }