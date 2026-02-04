//! Network service trait.

/// Network status service.
pub trait NetworkService: Send + Sync {
    fn start_monitoring(&self) -> Result<(), String> {
        Err("onNetworkStatusChange:fail not supported".to_string())
    }

    fn stop_monitoring(&self) -> Result<(), String> {
        Err("offNetworkStatusChange:fail not supported".to_string())
    }

    /// Get network type JSON: `{"networkType": "wifi", "isConnected": true}`
    fn get_network_type_json(&self) -> Result<String, String> {
        Err("getNetworkType:fail not supported".to_string())
    }

    /// Get local IP JSON: `{"localip": "192.168.1.1", "netmask": "255.255.255.0"}`
    fn get_local_ip_json(&self) -> Result<String, String> {
        Err("getLocalIPAddress:fail not supported".to_string())
    }
}
