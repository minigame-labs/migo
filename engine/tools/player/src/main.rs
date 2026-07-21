//! Linux dev player for the Migo engine.
//!
//! Drives the full engine (V8 + graphics + JS game) on
//! `x86_64-unknown-linux-gnu`, either offscreen (pbuffer, no window server) or
//! in a real X11 window. The game's own telemetry (`console.error` lines)
//! proves rendering, exactly like the on-device bench harness.
//!
//! Usage:
//!   migo-player [GAME_BUNDLE_DIR] [SECONDS] [--window]
//!
//! GAME_BUNDLE_DIR must contain `game.json` + `game.js` (a wx-style minigame
//! bundle). Defaults to the sibling migo-bench bunnymark bundle.
//!
//! `--window` (or `MIGO_PLAYER_WINDOW=1`) exercises the onscreen X11 presenter.
//! Headless stays the default so CI and the PNG capture path are unaffected.

mod x11_window;

use std::{path::PathBuf, sync::Arc, thread, time::Duration};

use migo_core::{PlatformServices, send_command_to_host, shutdown_host, spawn_host_thread};
use platform::desktop::platform::DesktopPlatform;
use platform::desktop::presenter::{
    LinuxOffscreenSurface, LinuxX11Surface, linux_graphics_platform, linux_x11_graphics_platform,
};
use shared::{config::InitOptions, protocol::host_cmd::HostCommand, surface::SurfaceRef};

use x11_window::X11Window;

const GAME_ID: &str = "player-demo";
const ENTRY: &str = "game.js";
const SURFACE_W: u32 = 720;
const SURFACE_H: u32 = 1280;
/// How often the window mode drains X events while the game renders.
const EVENT_POLL: Duration = Duration::from_millis(16);

fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(true)
        .init();

    let mut positional = Vec::new();
    let mut windowed = std::env::var_os("MIGO_PLAYER_WINDOW").is_some_and(|v| v != "0");
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--window" => windowed = true,
            "--offscreen" => windowed = false,
            _ => positional.push(arg),
        }
    }
    let bundle_dir = positional
        .first()
        .filter(|arg| !arg.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from("/home/xg/wkspace/migo-bench/shells/migo-shell/app/src/main/assets/game")
        });
    let secs: u64 = positional.get(1).and_then(|s| s.parse().ok()).unwrap_or(8);

    if let Err(err) = run(&bundle_dir, secs, windowed) {
        tracing::error!("player failed: {err}");
        std::process::exit(1);
    }
}

fn run(bundle_dir: &PathBuf, secs: u64, windowed: bool) -> Result<(), String> {
    // ---- Scratch dirs (files / cache / code cache) ----
    let root = std::env::temp_dir().join(format!("migo-player-{}", std::process::id()));
    let files_dir = root.join("files");
    let cache_dir = root.join("cache");
    let code_cache_dir = root.join("code-cache");

    // Deploy the game bundle into files_dir/migo/games/<id>/code/.
    let code_dir = files_dir
        .join("migo")
        .join("games")
        .join(GAME_ID)
        .join("code");
    std::fs::create_dir_all(&code_dir).map_err(|e| format!("mkdir code_dir: {e}"))?;
    for name in ["game.json", "game.js"] {
        let src = bundle_dir.join(name);
        let dst = code_dir.join(name);
        std::fs::copy(&src, &dst)
            .map_err(|e| format!("copy {} -> {}: {e}", src.display(), dst.display()))?;
    }
    std::fs::create_dir_all(&cache_dir).map_err(|e| format!("mkdir cache: {e}"))?;
    std::fs::create_dir_all(&code_cache_dir).map_err(|e| format!("mkdir code-cache: {e}"))?;
    tracing::info!("deployed game '{GAME_ID}' to {}", code_dir.display());

    // ---- InitOptions (signing off: dev player has no signed receipt) ----
    let opt = InitOptions::new()
        .with_files_dir(files_dir)
        .with_cache_dir(cache_dir)
        .with_code_cache_dir(code_cache_dir)
        .with_pixel_ratio(1.0)
        .with_target_fps(60)
        .with_debug_enabled(true)
        .with_code_signing_enabled(false);

    // ---- Render target: a real X11 window, or an offscreen pbuffer ----
    // The window (when present) must outlive the engine session: the render
    // thread holds an EGL surface built from its handles, so it is dropped only
    // after `shutdown_host` below.
    let mut window = if windowed {
        Some(X11Window::open("migo-player", SURFACE_W, SURFACE_H)?)
    } else {
        None
    };

    let (surface, graphics_platform) = match window.as_ref() {
        Some(window) => {
            let (width, height) = window.size();
            let surface: SurfaceRef = Arc::new(LinuxX11Surface::new(
                window.display(),
                window.window(),
                width,
                height,
            ));
            let platform = linux_x11_graphics_platform(window.display())
                .map_err(|e| format!("x11 graphics platform: {e:?}"))?;
            (surface, platform)
        }
        None => {
            let surface: SurfaceRef = Arc::new(LinuxOffscreenSurface::new(SURFACE_W, SURFACE_H));
            let platform =
                linux_graphics_platform().map_err(|e| format!("graphics platform: {e:?}"))?;
            (surface, platform)
        }
    };
    let host_kit: Arc<dyn PlatformServices> = Arc::new(DesktopPlatform::new());

    let mode = if windowed { "x11 window" } else { "offscreen" };
    tracing::info!("spawning host thread ({SURFACE_W}x{SURFACE_H} {mode})");
    let host_id = spawn_host_thread(surface, graphics_platform, host_kit, opt)
        .map_err(|e| format!("spawn_host_thread: {e:?}"))?;
    tracing::info!("host {host_id} spawned; loading game");

    send_command_to_host(
        host_id,
        HostCommand::EvaluateModule {
            game_id: GAME_ID.to_string(),
            entry: ENTRY.to_string(),
        },
    )
    .map_err(|e| format!("EvaluateModule: {e}"))?;

    // Capture the FIRST presented frame to PNG (proves the offscreen render
    // visually). Requested immediately so the render thread grabs an early
    // frame during the initial active-render burst — robust against the game
    // later going idle/erroring. The render thread fills FBO 0 (after the
    // DrawingBuffer blit, before eglSwapBuffers) exactly once.
    // Defaults into the run's scratch dir, not the working tree: running the
    // player from the repo root should not leave an untracked PNG behind.
    let png_path = std::env::var_os("MIGO_PLAYER_PNG")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("migo-player-frame.png"));
    graphics::frame_capture::request();
    // Let the game render for the window; the render thread keeps overwriting
    // the capture slot with the latest present, so early blank warmup frames
    // are superseded by frames containing game content.
    run_for(&mut window, Duration::from_secs(secs.max(4)));
    match graphics::frame_capture::take() {
        Some(frame) => {
            write_png(&png_path, &frame)?;
            tracing::info!(
                "captured {}x{} frame -> {}",
                frame.width,
                frame.height,
                png_path.display()
            );
        }
        None => tracing::warn!("frame capture: no frame was presented during the window"),
    }

    tracing::info!("shutting down host {host_id}");
    shutdown_host(host_id).map_err(|e| format!("shutdown_host: {e}"))?;
    thread::sleep(Duration::from_millis(300));
    // Only now may the window go: the render thread has stopped touching the
    // EGL surface built from its handles.
    drop(window);
    tracing::info!("player done");
    Ok(())
}

/// Wait for `total`, servicing window events when running windowed.
///
/// Returns early if the window manager asks the window to close, so the caller
/// still runs its capture + shutdown path instead of being killed mid-frame.
fn run_for(window: &mut Option<X11Window>, total: Duration) {
    let Some(window) = window.as_mut() else {
        thread::sleep(total);
        return;
    };
    let deadline = std::time::Instant::now() + total;
    while std::time::Instant::now() < deadline {
        if !window.pump() {
            tracing::info!("window close requested; stopping early");
            return;
        }
        thread::sleep(EVENT_POLL);
    }
}

/// Flip GL bottom-up rows to top-down and encode an RGBA8 PNG.
fn write_png(
    path: &std::path::Path,
    frame: &graphics::frame_capture::CapturedFrame,
) -> Result<(), String> {
    let (w, h) = (frame.width as usize, frame.height as usize);
    let stride = w * 4;
    if frame.rgba_bottom_up.len() != stride * h {
        return Err(format!(
            "capture size mismatch: {} != {}x{}x4",
            frame.rgba_bottom_up.len(),
            w,
            h
        ));
    }
    let mut top_down = vec![0u8; frame.rgba_bottom_up.len()];
    for y in 0..h {
        let src = &frame.rgba_bottom_up[(h - 1 - y) * stride..(h - y) * stride];
        top_down[y * stride..(y + 1) * stride].copy_from_slice(src);
    }
    let file =
        std::fs::File::create(path).map_err(|e| format!("create {}: {e}", path.display()))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), frame.width, frame.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|e| format!("png header: {e}"))?;
    writer
        .write_image_data(&top_down)
        .map_err(|e| format!("png data: {e}"))?;
    Ok(())
}
