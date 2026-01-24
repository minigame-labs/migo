

// Mock
function openAppAuthorizeSetting({ success, fail, complete } = {}) {
    success && success({ "code": 0, "message": "App authorization settings opened successfully" });
    // fail && fail();
    complete && complete({ "code": 0, "message": "App authorization settings operation completed" });
}

export { openAppAuthorizeSetting }