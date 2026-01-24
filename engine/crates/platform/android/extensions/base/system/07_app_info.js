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

export { getAppBaseInfo };