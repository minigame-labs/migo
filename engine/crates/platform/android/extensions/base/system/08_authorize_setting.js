import { op_get_app_authorization_setting } from "ext:core/ops";

function getAppAuthorizeSetting() {
    try {
        const jsonString = op_get_app_authorization_setting();
        const result = JSON.parse(jsonString);
        
        return result;
    } catch (error) {
        console.error("Failed to get app authorization setting:", error);
        
        const errorResult = {
            errMsg: "getAppAuthorizeSetting:fail " + error.message,
            albumAuthorized: "not determined",
            bluetoothAuthorized: "not determined",
            cameraAuthorized: "not determined",
            locationAuthorized: "not determined",
            locationReducedAccuracy: false,
            microphoneAuthorized: "not determined",
            notificationAuthorized: "not determined",
            notificationAlertAuthorized: "not determined",
            notificationBadgeAuthorized: "not determined",
            notificationSoundAuthorized: "not determined",
            phoneCalendarAuthorized: "not determined"
        };
 
        return errorResult;
    }
}

export { getAppAuthorizeSetting };