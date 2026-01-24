import { op_get_device_info } from "ext:core/ops";

function getDeviceInfo() {
    try {
        const jsonString = op_get_device_info();
        return JSON.parse(jsonString);
    } catch (error) {
        console.error("Failed to get device info:", error);
        return {
            abi: "",
            deviceAbi: "",
            benchmarkLevel: 25,
            brand: "",
            model: "unknown",
            system: "",
            platform: "android",
            cpuType: "",
            memorySize: ""
        };
    }
}

export { getDeviceInfo };