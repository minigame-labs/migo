//! Network service trait.

use crate::protocol::error::ServiceError;

/// Network status service.
pub trait NetworkService: Send + Sync {
    fn start_monitoring(&self) -> Result<(), ServiceError> {
        Err(ServiceError::not_supported("onNetworkStatusChange:fail not supported"))
    }

    fn stop_monitoring(&self) -> Result<(), ServiceError> {
        Err(ServiceError::not_supported("offNetworkStatusChange:fail not supported"))
    }

    /// Get network type JSON: `{"networkType": "wifi", "isConnected": true}`
    fn get_network_type_json(&self) -> Result<String, ServiceError> {
        Err(ServiceError::not_supported("getNetworkType:fail not supported"))
    }

    /// Get local IP JSON: `{"localip": "192.168.1.1", "netmask": "255.255.255.0"}`
    fn get_local_ip_json(&self) -> Result<String, ServiceError> {
        Err(ServiceError::not_supported("getLocalIPAddress:fail not supported"))
    }
}
