//! FigCraft Android ARM64 Tablet Shell (V2).
//!
//! V2 fixes:
//! - Uses `event_loop_builder` (`with_android_app`) for modern `eframe`/`winit` compatibility.
//! - Keeps `unsafe` strictly confined to `platform.rs` and the `android_main` FFI export.

pub mod mcp_http;
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
pub extern "C" fn android_main(app: AndroidApp) {
    use winit::platform::android::EventLoopBuilderExtAndroid;

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
        event_loop_builder: Some(Box::new(move |builder| {
            builder.with_android_app(app_for_loop);
        })),
        ..Default::default()
    };

    let run_result = eframe::run_native(
        "FigCraft",
        native_options,
        Box::new(move |cc| Ok(Box::new(shell::FigcraftAndroidShell::new(cc, app)))),
    );

    if let Err(err) = run_result {
        log::error!("eframe exited with error: {err}");
    }
}
