use crate::protocol::error::ServiceError;

/// Video playback service for creating and managing video players.
///
/// Follows Pattern C+D hybrid: methods are async (EvalScript callbacks)
/// and events are pushed via HostCommand variants.
pub trait VideoService: Send + Sync {
    /// Create a video player instance. Returns JSON with `{ videoId }`.
    fn create(&self, options_json: &str) -> Result<String, ServiceError>;
    /// Start playback.
    fn play(&self, video_id: u32) -> Result<(), ServiceError>;
    /// Pause playback.
    fn pause(&self, video_id: u32) -> Result<(), ServiceError>;
    /// Stop playback and reset to beginning.
    fn stop(&self, video_id: u32) -> Result<(), ServiceError>;
    /// Seek to position in seconds.
    fn seek(&self, video_id: u32, position: f64) -> Result<(), ServiceError>;
    /// Enter fullscreen mode.
    fn request_fullscreen(&self, video_id: u32, direction: i32) -> Result<(), ServiceError>;
    /// Exit fullscreen mode.
    fn exit_fullscreen(&self, video_id: u32) -> Result<(), ServiceError>;
    /// Update video properties (src, muted, loop, playbackRate, objectFit, size/position).
    fn set_property(&self, video_id: u32, property_json: &str) -> Result<(), ServiceError>;
    /// Destroy a video player instance and release resources.
    fn destroy(&self, video_id: u32) -> Result<(), ServiceError>;
}
