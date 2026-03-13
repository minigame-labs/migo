/// Barcode/QR code scanning service.
pub trait ScanCodeService: Send + Sync {
    /// Launch barcode scanner UI.
    ///
    /// JSON fields:
    /// - `onlyFromCamera`: bool (default false) - only scan from camera, not album
    /// - `scanType`: array of strings (default ["barCode","qrCode"]) - scan types to support
    ///   Valid values: "barCode", "qrCode", "datamatrix", "pdf417"
    ///
    /// Result is delivered asynchronously via `HostCommand::EvalScript`
    /// calling `_internalOnScanCodeResult(json)`.
    fn scan_code(&self, options_json: &str) -> Result<(), String> {
        Err("scanCode:fail not supported".to_string())
    }
}
