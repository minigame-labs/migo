class AppLifecycleManager {
    constructor() {
        this.onShowListeners = [];
        this.onHideListeners = [];
        this.onErrorListeners = [];
        this.isVisible = false;
        this.launchOptions = null;

        this._loadLaunchOptions();
    }

    _loadLaunchOptions() {
        try {
            this.launchOptions = {
                scene: 1001,
                query: {},
                shareTicket: "",
                referrerInfo: {
                    appId: "",
                    extraData: {},
                    chatType: 0
                }
            };
        } catch (error) {
            console.error("Failed to load launch options:", error);
            this.launchOptions = {
                scene: 1001,
                query: {},
                shareTicket: "",
                referrerInfo: {
                    appId: "",
                    extraData: {},
                    chatType: 0
                }
            };
        }
    }

    onShow(listener) {
        if (typeof listener === 'function') {
            this.onShowListeners.push(listener);
        }
    }

    onHide(listener) {
        if (typeof listener === 'function') {
            this.onHideListeners.push(listener);
        }
    }

    _triggerShow(options) {
        this.isVisible = true;
        const showOptions = options || this.launchOptions;

        console.log("Triggering onShow event with options:", showOptions);

        this.onShowListeners.forEach(listener => {
            try {
                listener(showOptions);
            } catch (error) {
                console.error("onShow listener error:", error);
            }
        });
    }

    _triggerHide() {
        this.isVisible = false;

        console.log("Triggering onHide event");

        this.onHideListeners.forEach(listener => {
            try {
                listener();
            } catch (error) {
                console.error("onHide listener error:", error);
            }
        });
    }

    getLaunchOptionsSync() {
        return this.launchOptions;
    }
}

const lifecycleManager = new AppLifecycleManager();

function onShow(listener) {
    lifecycleManager.onShow(listener);
}

function onHide(listener) {
    lifecycleManager.onHide(listener);
}

function getLaunchOptionsSync() {
    return lifecycleManager.getLaunchOptionsSync();
}

function getEnterOptionsSync() {
    return lifecycleManager.getLaunchOptionsSync();
}

function _internalTriggerOnShow(option) {
    lifecycleManager._triggerShow(option);
}

function _internalTriggerOnHide() {
    lifecycleManager._triggerHide();
}

export { onShow, onHide, getLaunchOptionsSync, getEnterOptionsSync, _internalTriggerOnHide, _internalTriggerOnShow };