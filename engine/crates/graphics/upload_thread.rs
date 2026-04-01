//! Dedicated texture upload thread with a shared EGL context.
//!
//! Runs GL texture uploads (PBO or AHB→EGLImage) off the render thread so
//! that `glTexImage2D` / EGLImage import never stalls frame presentation.
//!
//! **TierA only** — requires ES 3.0+ with working fence sync and a stable
//! shared EGL context (probed at init).  TierB devices skip this entirely
//! and upload on the render thread as before.

use crossbeam_channel::{bounded, Receiver, Sender};
use glow::HasContext;
use shared::error::ErrorCode;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tracing::{debug, info, warn};

/// A texture upload job sent from IO/render to the upload thread.
pub struct UploadJob {
    /// Unique image identifier (matches the host-side image_id).
    pub image_id: u64,
    pub width: u32,
    pub height: u32,
    /// RGBA pixel data.  Consumed (moved) by the upload thread.
    pub rgba: Arc<Vec<u8>>,
}

/// A completed upload, ready for the render thread to consume.
pub struct CompletedUpload {
    pub image_id: u64,
    pub texture: glow::NativeTexture,
    pub width: u32,
    pub height: u32,
    /// GL fence inserted after the upload.  The render thread must
    /// `glClientWaitSync` before sampling this texture.
    pub fence: glow::NativeFence,
}

// Safety: CompletedUpload is created on the upload thread (shared GL context)
// and consumed on the render thread (same EGLDisplay).  The GL object names
// (texture, fence) are valid across shared contexts by EGL spec, so
// transferring the raw handles between these two threads is sound.
unsafe impl Send for CompletedUpload {}

/// Handle to the upload thread.  Dropped when the engine shuts down.
pub struct UploadThreadHandle {
    job_tx: Sender<UploadJob>,
    result_rx: Receiver<CompletedUpload>,
    consecutive_failures: Arc<AtomicU32>,
    degraded: bool,
    _handle: std::thread::JoinHandle<()>,
}

/// Maximum consecutive upload failures before permanent degradation.
const MAX_FAILURES_BEFORE_DEGRADE: u32 = 3;

/// Queue capacity for pending upload jobs.
const JOB_QUEUE_CAPACITY: usize = 16;

impl UploadThreadHandle {
    /// Try to spawn the upload thread with a shared EGL context.
    ///
    /// Returns `None` if the shared context cannot be created (TierB device).
    /// The caller should fall back to render-thread uploads in that case.
    ///
    /// # Arguments
    /// * `egl` — EGL instance (dynamic)
    /// * `display` — current EGL display
    /// * `config` — EGL config used by the render context
    /// * `share_ctx` — the render thread's EGL context to share with
    pub fn try_spawn(
        egl: &khronos_egl::DynamicInstance<khronos_egl::EGL1_4>,
        display: khronos_egl::Display,
        config: khronos_egl::Config,
        share_ctx: khronos_egl::Context,
    ) -> Option<Self> {
        // Create shared context + 1x1 pbuffer for make_current.
        let ctx_attribs = [
            khronos_egl::CONTEXT_CLIENT_VERSION as i32,
            2,
            khronos_egl::NONE as i32,
        ];

        let shared_ctx = egl
            .create_context(display, config, Some(share_ctx), &ctx_attribs)
            .map_err(|e| warn!("Upload thread: eglCreateContext failed: {e:?}"))
            .ok()?;

        let pbuf_attribs = [
            khronos_egl::WIDTH as i32,
            1,
            khronos_egl::HEIGHT as i32,
            1,
            khronos_egl::NONE as i32,
        ];
        let pbuf = egl
            .create_pbuffer_surface(display, config, &pbuf_attribs)
            .map_err(|e| {
                warn!("Upload thread: eglCreatePbufferSurface failed: {e:?}");
                egl.destroy_context(display, shared_ctx).ok();
            })
            .ok()?;

        let (job_tx, job_rx) = bounded::<UploadJob>(JOB_QUEUE_CAPACITY);
        let (result_tx, result_rx) = bounded::<CompletedUpload>(JOB_QUEUE_CAPACITY);
        let consecutive_failures = Arc::new(AtomicU32::new(0));
        let failures_clone = consecutive_failures.clone();

        // We need raw pointers to pass to the thread because EGL types are
        // not Send.  The upload thread will reconstruct EGL/GL wrappers.
        let egl_lib_path = "libEGL.so";
        let display_raw = display.as_ptr() as usize;
        let ctx_raw = shared_ctx.as_ptr() as usize;
        let pbuf_raw = pbuf.as_ptr() as usize;

        let handle = std::thread::Builder::new()
            .name("Migo-Upload".into())
            .spawn(move || {
                shared::thread_priority::set_current_thread_priority(
                    shared::thread_priority::Priority::Default,
                );
                upload_thread_main(
                    egl_lib_path,
                    display_raw,
                    ctx_raw,
                    pbuf_raw,
                    job_rx,
                    result_tx,
                    failures_clone,
                );
            })
            .map_err(|e| warn!("Upload thread: spawn failed: {e}"))
            .ok()?;

        info!("Upload thread spawned (shared GL context)");

        Some(Self {
            job_tx,
            result_rx,
            consecutive_failures,
            degraded: false,
            _handle: handle,
        })
    }

    /// Submit an upload job.  Returns false if the thread has degraded.
    pub fn submit(&self, job: UploadJob) -> bool {
        if self.degraded {
            return false;
        }
        self.job_tx.try_send(job).is_ok()
    }

    /// Drain completed uploads.  Non-blocking — returns whatever is ready.
    pub fn drain_completed(&mut self, out: &mut Vec<CompletedUpload>) {
        while let Ok(result) = self.result_rx.try_recv() {
            self.consecutive_failures.store(0, Ordering::Relaxed);
            out.push(result);
        }
    }

    /// Check if the upload thread has permanently degraded.
    pub fn is_degraded(&self) -> bool {
        self.degraded
            || self.consecutive_failures.load(Ordering::Relaxed) >= MAX_FAILURES_BEFORE_DEGRADE
    }
}

// ---------------------------------------------------------------------------
// Upload thread main loop
// ---------------------------------------------------------------------------

fn upload_thread_main(
    egl_lib_path: &str,
    display_raw: usize,
    ctx_raw: usize,
    pbuf_raw: usize,
    job_rx: Receiver<UploadJob>,
    result_tx: Sender<CompletedUpload>,
    failures: Arc<AtomicU32>,
) {
    // Reconstruct EGL instance on this thread.
    let egl = match unsafe { khronos_egl::DynamicInstance::<khronos_egl::EGL1_4>::load_required() }
    {
        Ok(e) => e,
        Err(e) => {
            warn!("Upload thread: failed to load EGL: {e:?}");
            return;
        }
    };

    // Reconstruct opaque handles from raw pointers.
    let display = unsafe { khronos_egl::Display::from_ptr(display_raw as *mut _) };
    let ctx = unsafe { khronos_egl::Context::from_ptr(ctx_raw as *mut _) };
    let pbuf = unsafe { khronos_egl::Surface::from_ptr(pbuf_raw as *mut _) };

    // Make the shared context current on this thread.
    if egl
        .make_current(display, Some(pbuf), Some(pbuf), Some(ctx))
        .is_err()
    {
        warn!("Upload thread: eglMakeCurrent failed");
        return;
    }

    let gl = unsafe {
        glow::Context::from_loader_function(|s| {
            egl.get_proc_address(s)
                .map(|f| f as *const std::ffi::c_void)
                .unwrap_or(std::ptr::null())
        })
    };

    info!("Upload thread: GL context ready");

    // Process jobs until channel closes.
    while let Ok(job) = job_rx.recv() {
        match do_upload(&gl, &job) {
            Ok(completed) => {
                failures.store(0, Ordering::Relaxed);
                if let Err(e) = result_tx.try_send(completed) {
                    // Channel full or disconnected — clean up GL resources
                    // to prevent GPU memory leak.
                    let c = e.into_inner();
                    warn!(
                        "Upload thread: result channel unavailable, deleting texture for image_id={}",
                        c.image_id
                    );
                    unsafe {
                        gl.delete_texture(c.texture);
                        gl.delete_sync(c.fence);
                    }
                }
            }
            Err(e) => {
                let count = failures.fetch_add(1, Ordering::Relaxed) + 1;
                warn!("Upload thread: upload failed ({count}x): {e}");
                if count >= MAX_FAILURES_BEFORE_DEGRADE {
                    warn!("Upload thread: degraded after {count} consecutive failures");
                    break;
                }
            }
        }
    }

    // Cleanup.
    egl.make_current(display, None, None, None).ok();
    egl.destroy_context(display, ctx).ok();
    egl.destroy_surface(display, pbuf).ok();
    info!("Upload thread: exited");
}

/// Perform a single texture upload on the shared GL context.
fn do_upload(gl: &glow::Context, job: &UploadJob) -> Result<CompletedUpload, String> {
    unsafe {
        let tex = gl
            .create_texture()
            .map_err(|e| format!("create_texture: {e}"))?;

        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_WRAP_S,
            glow::CLAMP_TO_EDGE as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_WRAP_T,
            glow::CLAMP_TO_EDGE as i32,
        );

        // Upload RGBA data via PBO for async DMA.
        let pbo = gl
            .create_buffer()
            .map_err(|e| format!("create_buffer(PBO): {e}"))?;
        gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, Some(pbo));
        gl.buffer_data_u8_slice(glow::PIXEL_UNPACK_BUFFER, &job.rgba, glow::STREAM_DRAW);
        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);

        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA as i32,
            job.width as i32,
            job.height as i32,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::BufferOffset(0),
        );

        gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, None);
        gl.delete_buffer(pbo);

        // Insert fence so the render thread knows when the upload is complete.
        let fence = gl
            .fence_sync(glow::SYNC_GPU_COMMANDS_COMPLETE, 0)
            .map_err(|e| format!("fence_sync: {e}"))?;

        // Flush to ensure the fence and upload are submitted to the GPU.
        gl.flush();

        gl.bind_texture(glow::TEXTURE_2D, None);

        let err = gl.get_error();
        if err != glow::NO_ERROR {
            gl.delete_texture(tex);
            gl.delete_sync(fence);
            return Err(format!("GL error after upload: 0x{err:X}"));
        }

        debug!(
            "Upload thread: uploaded image_id={} {}x{} ({} bytes)",
            job.image_id,
            job.width,
            job.height,
            job.rgba.len()
        );

        Ok(CompletedUpload {
            image_id: job.image_id,
            texture: tex,
            width: job.width,
            height: job.height,
            fence,
        })
    }
}
