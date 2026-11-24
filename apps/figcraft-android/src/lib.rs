//! FigCraft Android ARM64 Tablet Shell (V2).
//!
//! V2 fixes:
//! - Uses `event_loop_builder` (`with_android_app`) for modern `eframe`/`winit` compatibility.
//! - Keeps `unsafe` strictly confined to `platform.rs` and the `android_main` FFI export.

pub mod agent_bubble;
pub mod mcp_http;
pub mod ngrok_tunnel;
pub mod platform;
pub mod protocol;
pub mod services;
pub mod shell;
pub mod touch;

#[cfg(target_os = "android")]
use android_activity::AndroidApp;

/// Android native entry point invoked by `GameActivity`.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("FigCraft"),
    );

    services::install_panic_hook(&app);

    if let Err(err) = platform::init_android_bridge(&app) {
        log::error!("Failed to initialize FigCraft JNI bridge: {err}");
    }

    let app_for_loop = app.clone();
    let native_options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        android_app: Some(app_for_loop),
        ..Default::default()
    };

    let unwind_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        eframe::run_native(
            "FigCraft",
            native_options,
            Box::new(move |cc| Ok(Box::new(shell::FigcraftAndroidShell::new(cc, app)))),
        )
    }));

    match unwind_result {
        Ok(Ok(())) => {}
        Ok(Err(err)) => {
            let msg = format!("eframe exited with error: {err}");
            log::error!("{msg}");
            let _ = platform::call_activity("save_crash_log", &msg);
        }
        Err(_) => {
            log::error!("android_main caught panic unwind (logged by panic hook)");
        }
    }
}
