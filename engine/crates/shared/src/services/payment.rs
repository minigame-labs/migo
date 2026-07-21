//! Payment service traits for Midas payment operations.

use crate::protocol::error::ServiceError;

/// Payment service for in-app purchases.
pub trait PaymentService: Send + Sync {
    /// Check if the current environment supports Midas payment (Mode B, sync).
    ///
    /// Returns JSON: `{"data":{"allow_pay":true/false}}`
    fn check_is_support_midas_payment(&self, _options_json: &str) -> Result<String, ServiceError> {
        Ok(r#"{"data":{"allow_pay":false}}"#.to_string())
    }

    /// Trigger Midas payment flow (Mode C, async).
    ///
    /// JSON fields (input):
    /// - `mode`: string
    /// - `env`: number
    /// - `offerId`: string
    /// - `currencyType`: string
    /// - `platform`: string
    /// - `buyQuantity`: number
    /// - `zoneId`: number
    /// - `outTradeNo`: string
    ///
    /// Result delivered via `onMidasPaymentResult` callback.
    fn request_midas_payment(&self, _options_json: &str) -> Result<(), ServiceError> {
        Err(ServiceError::not_supported(
            "requestMidasPayment:fail not supported",
        ))
    }

    /// Trigger Midas payment for game items (Mode C, async).
    ///
    /// JSON fields (input):
    /// - `signData`: string
    /// - `paySig`: string
    /// - `signature`: string
    ///
    /// Result delivered via `onMidasPaymentGameItemResult` callback.
    fn request_midas_payment_game_item(&self, _options_json: &str) -> Result<(), ServiceError> {
        Err(ServiceError::not_supported(
            "requestMidasPaymentGameItem:fail not supported",
        ))
    }
}
