class UpdateManager {
    constructor() {
        this.checkForUpdateListeners = [];
        this.updateReadyListeners = [];
        this.updateFailedListeners = [];
        this.hasUpdate = false;
        this.isReady = false;
        
        setTimeout(() => {
            this._simulateUpdateCheck();
        }, 1000);
    }
    
    onCheckForUpdate(listener) {
        if (typeof listener === 'function') {
            this.checkForUpdateListeners.push(listener);
        }
    }
    
    onUpdateReady(listener) {
        if (typeof listener === 'function') {
            this.updateReadyListeners.push(listener);
        }
    }
    
    onUpdateFailed(listener) {
        if (typeof listener === 'function') {
            this.updateFailedListeners.push(listener);
        }
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
        
        this.checkForUpdateListeners.forEach(listener => {
            try {
                listener({ hasUpdate });
            } catch (error) {
                console.error('UpdateManager onCheckForUpdate listener error:', error);
            }
        });
        
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
                
                this.updateReadyListeners.forEach(listener => {
                    try {
                        listener();
                    } catch (error) {
                        console.error('UpdateManager onUpdateReady listener error:', error);
                    }
                });
            } else {
                console.log('UpdateManager: Update download failed');
                
                this.updateFailedListeners.forEach(listener => {
                    try {
                        listener();
                    } catch (error) {
                        console.error('UpdateManager onUpdateFailed listener error:', error);
                    }
                });
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

export { getUpdateManager, UpdateManager };