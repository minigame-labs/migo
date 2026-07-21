import { createListenerGroup } from "ext:host_v8_base/02_async.js";

class UpdateManager {
    constructor() {
        this.checkForUpdateListeners = createListenerGroup('UpdateManager onCheckForUpdate');
        this.updateReadyListeners = createListenerGroup('UpdateManager onUpdateReady');
        this.updateFailedListeners = createListenerGroup('UpdateManager onUpdateFailed');
        this.hasUpdate = false;
        this.isReady = false;
        
        setTimeout(() => {
            this._simulateUpdateCheck();
        }, 1000);
    }
    
    onCheckForUpdate(listener) {
        this.checkForUpdateListeners.on(listener);
    }
    
    onUpdateReady(listener) {
        this.updateReadyListeners.on(listener);
    }
    
    onUpdateFailed(listener) {
        this.updateFailedListeners.on(listener);
    }
    
    applyUpdate() {
        if (!this.isReady) {
            console.warn('UpdateManager.applyUpdate: No update is ready to apply');
            return;
        }
        
        console.log('UpdateManager.applyUpdate: Applying update and restarting...');
        
        setTimeout(() => {
            console.log('UpdateManager: Application restarted with new version');
            this.hasUpdate = false;
            this.isReady = false;
        }, 500);
    }

    _simulateUpdateCheck() {
        console.log('UpdateManager: Checking for updates...');
        
        const hasUpdate = Math.random() < 0.3;
        this.hasUpdate = hasUpdate;
        
        this.checkForUpdateListeners.trigger({ hasUpdate });
        
        if (hasUpdate) {
            this._simulateDownload();
        }
    }
    
    _simulateDownload() {
        console.log('UpdateManager: Downloading update...');
        
        const downloadTime = 2000 + Math.random() * 3000;
        
        setTimeout(() => {
            const success = Math.random() < 0.9;
            
            if (success) {
                this.isReady = true;
                console.log('UpdateManager: Update download completed');
                
                this.updateReadyListeners.trigger();
            } else {
                console.log('UpdateManager: Update download failed');

                this.updateFailedListeners.trigger();
            }
        }, downloadTime);
    }
}

let updateManagerInstance = null;

function getUpdateManager() {
    if (!updateManagerInstance) {
        updateManagerInstance = new UpdateManager();
    }
    return updateManagerInstance;
}

function checkUpdate(options = {}) {
    const { success, fail, complete } = options;
    try {
        const result = {
            hasUpdate: false,
            errMsg: 'checkUpdate:ok',
        };
        if (typeof success === 'function') {
            success(result);
        }
        if (typeof complete === 'function') {
            complete(result);
        }
        return Promise.resolve(result);
    } catch (error) {
        const errorResult = {
            errMsg: 'checkUpdate:fail ' + error.message,
        };
        if (typeof fail === 'function') {
            fail(errorResult);
        }
        if (typeof complete === 'function') {
            complete(errorResult);
        }
        return Promise.reject(errorResult);
    }
}

export { getUpdateManager, UpdateManager, checkUpdate };
