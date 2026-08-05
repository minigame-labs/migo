use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::LazyLock;

use parking_lot::Mutex;

use crate::protocol::error::ServiceError;

/// Subpackage download service.
///
/// Handles downloading subpackages from the host app's CDN.
/// The JS layer resolves subpackage names to root paths from injected
/// runtime config metadata, then delegates the actual download to the platform.
///
/// JSON fields passed to both methods:
/// - `requestId`: number -- unique ID for correlating progress/result callbacks
/// - `name`: string -- subpackage name (or `"__GAME__"` for the main package)
/// - `root`: string -- normalized root path relative to code_dir (e.g. `"subpackages/stage1"`)
///
/// The platform should:
/// 1. Download the subpackage as a zip file, to a location of its own choosing
/// 2. Report progress via `NativeMethods.onSubpackageProgress(sessionId, json)`
///    where json = `{"requestId":N,"progress":50,"totalBytesWritten":1024,"totalBytesExpectedToWrite":2048}`
/// 3. Report result via `NativeMethods.onSubpackageResult(sessionId, json)`
///    where json = `{"requestId":N,"zipPath":"/abs/path.zip"}` on success or
///    `{"requestId":N,"error":"reason"}` on failure
///
/// **That path must not reach the game.** It is a host path and the result travels
/// through the game's JS on its way back to the installer, so a platform's inbound
/// callback hands the payload to [`intercept_download_result`] before forwarding
/// it: the path stays here and the game names only the request it belongs to.
pub trait SubpackageService: Send + Sync {
    /// Trigger a subpackage download.
    ///
    /// Called by both `loadSubpackage` and `preDownloadSubpackage`.
    /// The JS layer handles code execution after download completes.
    fn download_subpackage(&self, _options_json: &str) -> Result<(), ServiceError> {
        Err(ServiceError::not_supported(
            "loadSubpackage:fail not supported",
        ))
    }
}

/// Zip paths the host reported for downloads it performed, per session.
///
/// Keyed by session as well as by request, so one session's request number cannot
/// name another's download. Entries are consumed by the install that ingests them
/// and dropped wholesale at session teardown, which is what bounds this: a
/// download whose result the JS layer never installs would otherwise sit here for
/// the life of the session.
static DOWNLOADED_ZIPS: LazyLock<Mutex<HashMap<(i32, u64), PathBuf>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Take the zip path the host reported for `request_id`, if it reported one.
///
/// One shot: the install consumes it, so a second attempt on the same request
/// finds nothing rather than re-ingesting a file the host may have cleaned up.
pub fn take_downloaded_zip(host_id: i32, request_id: u64) -> Option<PathBuf> {
    DOWNLOADED_ZIPS.lock().remove(&(host_id, request_id))
}

/// Drop every path recorded for `host_id`, at session teardown.
pub fn forget_downloaded_zips(host_id: i32) {
    DOWNLOADED_ZIPS.lock().retain(|(id, _), _| *id != host_id);
}

/// Record the host's zip path out of the game's reach, and return the payload the
/// game may see.
///
/// The game must not name the file an install ingests. The ingest reads a real
/// path, so any zip the process can read — the host app's own package among them —
/// would otherwise become readable through the game's own `/code`. `zipPath` is
/// therefore always removed, and recorded when it is a usable path for a request.
pub fn intercept_download_result(host_id: i32, result_json: &str) -> String {
    let Ok(mut payload) = serde_json::from_str::<serde_json::Value>(result_json) else {
        return result_json.to_string();
    };
    let Some(fields) = payload.as_object_mut() else {
        return result_json.to_string();
    };
    let reported = fields.remove("zipPath");
    let request_id = fields.get("requestId").and_then(serde_json::Value::as_u64);

    if let (Some(serde_json::Value::String(path)), Some(request_id)) = (reported, request_id) {
        if !path.is_empty() {
            DOWNLOADED_ZIPS
                .lock()
                .insert((host_id, request_id), PathBuf::from(path));
        }
    }

    serde_json::to_string(&payload).unwrap_or_else(|_| result_json.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_download_result_reaches_the_game_without_the_host_path() {
        let host = 8101;
        forget_downloaded_zips(host);

        let for_js = intercept_download_result(
            host,
            r#"{"requestId":3,"zipPath":"/data/app/host.apk","version":"2.0"}"#,
        );

        assert!(
            !for_js.contains("zipPath") && !for_js.contains("host.apk"),
            "the game must not be handed a path it could give the installer: {for_js}",
        );
        assert!(for_js.contains("\"requestId\":3"));
        assert!(for_js.contains("\"version\":\"2.0\""));
        assert_eq!(
            take_downloaded_zip(host, 3),
            Some(PathBuf::from("/data/app/host.apk")),
        );
    }

    #[test]
    fn one_sessions_request_number_cannot_name_anothers_download() {
        let (first, second) = (8102, 8103);
        forget_downloaded_zips(first);
        forget_downloaded_zips(second);

        intercept_download_result(first, r#"{"requestId":1,"zipPath":"/tmp/first.zip"}"#);
        intercept_download_result(second, r#"{"requestId":1,"zipPath":"/tmp/second.zip"}"#);

        assert_eq!(
            take_downloaded_zip(first, 1),
            Some(PathBuf::from("/tmp/first.zip")),
        );
        assert_eq!(
            take_downloaded_zip(second, 1),
            Some(PathBuf::from("/tmp/second.zip")),
        );
    }

    #[test]
    fn a_recorded_path_is_consumed_by_the_install_that_takes_it() {
        let host = 8104;
        forget_downloaded_zips(host);

        intercept_download_result(host, r#"{"requestId":7,"zipPath":"/tmp/stage.zip"}"#);

        assert!(take_downloaded_zip(host, 7).is_some());
        assert_eq!(take_downloaded_zip(host, 7), None);
    }

    #[test]
    fn a_failed_download_records_nothing_and_keeps_its_reason() {
        let host = 8105;
        forget_downloaded_zips(host);

        let for_js =
            intercept_download_result(host, r#"{"requestId":4,"error":"connection reset"}"#);

        assert!(for_js.contains("connection reset"));
        assert_eq!(take_downloaded_zip(host, 4), None);
    }

    #[test]
    fn session_teardown_forgets_paths_it_never_installed() {
        let (host, other) = (8106, 8107);
        forget_downloaded_zips(host);
        forget_downloaded_zips(other);

        intercept_download_result(host, r#"{"requestId":1,"zipPath":"/tmp/abandoned.zip"}"#);
        intercept_download_result(other, r#"{"requestId":1,"zipPath":"/tmp/live.zip"}"#);

        forget_downloaded_zips(host);

        assert_eq!(take_downloaded_zip(host, 1), None);
        assert_eq!(
            take_downloaded_zip(other, 1),
            Some(PathBuf::from("/tmp/live.zip")),
            "teardown must drop one session's paths and leave another's",
        );
    }
}
