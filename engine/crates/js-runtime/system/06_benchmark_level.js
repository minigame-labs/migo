function getDeviceBenchmarkInfo(options = {}) {
    const { success, fail, complete } = options;

    const result = {
        benchmarkLevel: 26,
        modelLevel: 0
    };

    if (typeof success === 'function') {
        success(result);
    }

    if (typeof complete === 'function') {
        complete(result);
    }
}

export { getDeviceBenchmarkInfo };