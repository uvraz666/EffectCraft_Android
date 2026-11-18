//! EffectCraft on Android.
//!
//! The interface is the desktop one (`effectcraft-ui-egui`), unchanged: same docking panels, same
//! timeline, same shortcuts. This crate only replaces what the desktop app takes from the
//! operating system: the activity and window (winit's Android backend), the file dialogs (the
//! system document picker, [`picker`]), audio output (cpal's Android backend) and the settings
//! folder (the app's private storage). Everything the desktop app does for crash reports, the
//! launch guard, window geometry, the control channel and the native menu bar is left out.
//!
//! Desktop feel on a phone: the activity is locked to landscape and [`scale`] picks the egui zoom
//! so the screen is as many points wide as a desktop window (1280 by default), then Settings ▸
//! Appearance ▸ UI Scale multiplies that. Touch is egui's own: tap = click, long press = right
//! click, drag = drag. A Bluetooth mouse and keyboard work like on a desktop.
#![cfg(target_os = "android")]

mod audio_out;
mod picker;
mod scale;

use effectcraft_ui_egui::EffectcraftApp;
use serde_json::json;
use winit::platform::android::activity::AndroidApp;

/// The activity's entry point (`android-activity` calls it on the app's main thread).
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag("EffectCraft"));
    std::panic::set_hook(Box::new(|info| log::error!("panic: {info}")));
    prepare_environment(&app);
    effectcraft_engine::logging::install_panic_hook();
    log::info!("effectcraft {} (android): starting", env!("CARGO_PKG_VERSION"));

    // Fonts for the font menus are read while the window opens, as on the desktop.
    effectcraft_engine::text::fonts::scan_system_in_background();

    let options = eframe::NativeOptions {
        android_app: Some(app.clone()),
        wgpu_options: wgpu_options(),
        ..Default::default()
    };
    let host = app.clone();
    let result = eframe::run_native(
        "EffectCraft",
        options,
        Box::new(move |cc| {
            EffectcraftApp::install_ime_reset_hooks(&cc.egui_ctx);
            // One window per activity: floating panels and the video preview stay inside it, as
            // in the web build (the menus then also leave out "Move Panel to New Window").
            cc.egui_ctx.set_embed_viewports(true);
            if let Some(rs) = &cc.wgpu_render_state {
                log::info!("graphics: {:?}", rs.adapter.get_info());
            }
            let gpu_failures = effectcraft_ui_egui::gpu_failure::GpuFailureBridge::new(&cc.egui_ctx);
            if let Some(rs) = &cc.wgpu_render_state {
                let errors = gpu_failures.clone();
                rs.device.on_uncaptured_error(std::sync::Arc::new(move |error| {
                    let message = format!("uncaptured GPU error: {error}");
                    if errors.report(&message, false) {
                        log::error!("{message}");
                    }
                }));
                let lost = gpu_failures.clone();
                rs.device.set_device_lost_callback(move |reason, message| {
                    lost.report(&format!("GPU device lost ({reason:?}): {message}"), true);
                });
            }

            let mut session = effectcraft_host::session();
            session.check_footage_on_open = true;
            session.autosave.background = true;
            if let Some(dir) = effectcraft_host::config_dir() {
                let plugins = dir.join(effectcraft_engine::commands::PLUGINS_DIR);
                if plugins.is_dir() {
                    let _ = session.execute("effect.plugins.load", json!({"folder": plugins.to_string_lossy()}));
                    if let Err(e) = session.execute("library.loadInstalled", json!({})) {
                        log::warn!("plug-in library: {e}");
                    }
                }
                session.config = Some(std::sync::Arc::new(effectcraft_engine::config::DirConfig::new(dir)));
            }
            session.load_settings();
            let _ = session.execute("file.runStartupScripts", json!({}));
            let recovery = session.begin_recovery();
            let show_home = session.prefs.startup.show_home_on_launch;
            let _ = session.execute("window.restoreScriptPanels", json!({}));

            let mut app = EffectcraftApp::new(session);
            app.set_gpu_failure_bridge(gpu_failures);
            app.ui.start_screen = show_home;
            if let Some(r) = recovery {
                app.offer_recovery(r);
            }
            picker::install(&mut app, &host, &cc.egui_ctx);
            app.hooks.audio_device = Some(Box::new(audio_out::open));
            app.hooks.audio_devices = Some(Box::new(audio_out::devices));
            log::info!("app created");
            Ok(Box::new(Mobile { app, scale: scale::Governor::default() }))
        }),
    );
    if let Err(e) = result {
        log::error!("EffectCraft couldn't start its window: {e}");
    }
}

/// Where the engine keeps things, as the desktop app's environment would say: the settings folder
/// and a temp folder in the app's private storage, and a `HOME` for code that expects one.
fn prepare_environment(app: &AndroidApp) {
    let Some(files) = app.internal_data_path() else {
        log::warn!("no internal data path: settings will not be kept");
        return;
    };
    let cache = files.join("cache");
    let _ = std::fs::create_dir_all(&cache);
    // SAFETY: called once at the start of android_main, before any other thread of ours runs.
    unsafe {
        std::env::set_var("EFFECTCRAFT_CONFIG_DIR", files.join("EffectCraft"));
        std::env::set_var("HOME", &files);
        std::env::set_var("TMPDIR", &cache);
    }
}

/// The window device asks for the adapter's own limits (textures up to 16384 px), as on the
/// desktop; the compositor checks what it needs and falls back to the CPU when it can't run.
fn wgpu_options() -> eframe::WgpuConfiguration {
    use eframe::egui_wgpu::WgpuSetup;
    let mut config = eframe::WgpuConfiguration::default().with_surface_config(eframe::SurfaceConfig::LOW_LATENCY);
    if let WgpuSetup::CreateNew(setup) = &mut config.wgpu_setup {
        // Vulkan only: Android's GL path is far slower and the compositor's compute shaders need Vulkan.
        setup.instance_descriptor.backends = eframe::wgpu::Backends::VULKAN;
        setup.instance_descriptor.flags =
            setup.instance_descriptor.flags.difference(eframe::wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL).with_env();
        setup.device_descriptor = std::sync::Arc::new(|adapter| {
            let l = adapter.limits();
            eframe::wgpu::DeviceDescriptor {
                label: Some("EffectCraft device"),
                required_limits: eframe::wgpu::Limits { max_texture_dimension_2d: l.max_texture_dimension_2d.min(16384), ..l },
                memory_hints: eframe::wgpu::MemoryHints::MemoryUsage,
                ..Default::default()
            }
        });
    }
    config
}

/// The egui app plus the screen-scale governor.
struct Mobile {
    app: EffectcraftApp,
    scale: scale::Governor,
}

impl eframe::App for Mobile {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.app.tick(ctx, frame);
        self.app.finish_ime_logic(ctx);
        self.scale.apply(ctx, self.app.session.prefs.appearance.ui_scale);
        effectcraft_ui_egui::panels::unsaved::quit_on_close(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.app.ui(ui, frame);
    }

    fn on_exit(&mut self) {
        self.app.on_exit();
        effectcraft_host::stop_helpers();
        let _ = self.app.session.execute("file.runShutdownScripts", json!({}));
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.app.raw_input_hook(ctx, raw_input);
    }
}
