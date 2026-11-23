//! Main `eframe::App` wrapper hosting FigCraft's UI inside Android `GameActivity` (V6_hotfix3 · egui 0.37).

use crate::mcp_http::{EmbeddedMcpServer, McpServerConfig, PendingMcpRequest};
use crate::platform;
use crate::protocol::ActivityEvent;
use crate::services::{AndroidServices, AndroidShellPrefs};
use crate::touch::TabletInputState;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::mpsc;

const ZOOM_PRESETS: [f32; 3] = [1.0, 1.25, 1.4];

enum LoadedAsset {
    Document {
        display_name: String,
        bytes: Vec<u8>,
    },
    Image {
        display_name: String,
        mime: String,
        bytes: Vec<u8>,
    },
    Err {
        display_name: String,
        error: String,
    },
}

pub struct FigcraftAndroidShell {
    services: AndroidServices,
    prefs: AndroidShellPrefs,
    tablet_input: TabletInputState,
    mcp_server: Option<EmbeddedMcpServer>,
    mcp_lan_mode: bool,
    mcp_endpoint_label: String,
    status_banner: Option<String>,
    pending_paste_text: Option<String>,
    last_focused_id: Option<egui::Id>,
    restore_focus_before_paste: bool,
    asset_tx: mpsc::Sender<LoadedAsset>,
    asset_rx: mpsc::Receiver<LoadedAsset>,
    // Integration hook: replace with upstream `figcraft_ui_egui::FigcraftApp`
    // once dropped into the `storytold/figcraft` workspace:
    // inner: figcraft_ui_egui::FigcraftApp,
}

impl FigcraftAndroidShell {
    #[cfg(target_os = "android")]
    pub fn new(cc: &eframe::CreationContext<'_>, app: android_activity::AndroidApp) -> Self {
        let internal_dir = app
            .internal_data_path()
            .unwrap_or_else(|| PathBuf::from("/data/local/tmp/figcraft"));
        Self::init_common(cc, internal_dir)
    }

    #[cfg(not(target_os = "android"))]
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Self::init_common(cc, std::env::temp_dir().join("figcraft-android-sim"))
    }

    fn init_common(cc: &eframe::CreationContext<'_>, internal_dir: PathBuf) -> Self {
        let services = AndroidServices::new(internal_dir);
        let prefs = services.load_prefs();
        cc.egui_ctx.set_zoom_factor(prefs.ui_zoom_factor);
        if prefs.immersive_mode {
            services.set_immersive_mode(true);
        }

        let (asset_tx, asset_rx) = mpsc::channel();
        let mut status_banner = None;
        let mut mcp_endpoint_label = String::new();

        let mcp_server = match EmbeddedMcpServer::start(McpServerConfig::default()) {
            Ok(srv) => {
                mcp_endpoint_label = format!("http://{}/mcp", srv.config.bind_addr);
                services.set_mcp_foreground_service(true, &mcp_endpoint_label);
                status_banner = Some(format!("MCP: {mcp_endpoint_label}"));
                Some(srv)
            }
            Err(err) => {
                log::warn!("MCP server disabled: {err}");
                None
            }
        };

        Self {
            services,
            prefs,
            tablet_input: TabletInputState::default(),
            mcp_server,
            mcp_lan_mode: false,
            mcp_endpoint_label,
            status_banner,
            pending_paste_text: None,
            last_focused_id: None,
            restore_focus_before_paste: false,
            asset_tx,
            asset_rx,
        }
    }

    fn cycle_ui_zoom(&mut self, ctx: &egui::Context) {
        let next = ZOOM_PRESETS
            .iter()
            .copied()
            .find(|&z| z > self.prefs.ui_zoom_factor + 0.01)
            .unwrap_or(ZOOM_PRESETS[0]);
        self.prefs.ui_zoom_factor = next;
        ctx.set_zoom_factor(next);
        self.services.save_prefs(&self.prefs);
        self.status_banner = Some(format!("UI Scale: {}%", (next * 100.0).round() as i32));
    }

    fn toggle_immersive(&mut self) {
        self.prefs.immersive_mode = !self.prefs.immersive_mode;
        self.services.set_immersive_mode(self.prefs.immersive_mode);
        self.services.save_prefs(&self.prefs);
        self.status_banner = Some(if self.prefs.immersive_mode {
            "Immersive fullscreen ON".into()
        } else {
            "Immersive fullscreen OFF".into()
        });
    }

    fn trigger_paste(&mut self) {
        self.restore_focus_before_paste = true;
        let _ = self.services.request_paste_clipboard();
    }

    fn spawn_async_read(&self, path: String, display_name: String, image_mime: Option<String>) {
        let tx = self.asset_tx.clone();
        let _ = std::thread::Builder::new()
            .name("figcraft-read".into())
            .spawn(move || {
                let res = match std::fs::read(&path) {
                    Ok(bytes) => {
                        let _ = std::fs::remove_file(&path);
                        match image_mime {
                            Some(mime) => LoadedAsset::Image {
                                display_name,
                                mime,
                                bytes,
                            },
                            None => LoadedAsset::Document { display_name, bytes },
                        }
                    }
                    Err(err) => LoadedAsset::Err {
                        display_name,
                        error: err.to_string(),
                    },
                };
                let _ = tx.send(res);
            });
    }

    fn drain_activity_events(&mut self) {
        for event in platform::poll_activity_events() {
            self.tablet_input.handle_event(&event);
            match event {
                ActivityEvent::FileOpened { path, display_name } => {
                    self.status_banner = Some(format!("Loading {display_name}..."));
                    self.spawn_async_read(path, display_name, None);
                }
                ActivityEvent::ImagePicked {
                    path,
                    display_name,
                    mime,
                } => {
                    self.status_banner = Some(format!("Placing image {display_name}..."));
                    self.spawn_async_read(path, display_name, Some(mime));
                }
                ActivityEvent::ClipboardText(text) => {
                    self.pending_paste_text = Some(text);
                    self.status_banner = Some("Pasted text from clipboard".into());
                }
                ActivityEvent::ClipboardImage { path, mime } => {
                    self.status_banner = Some("Pasting image from clipboard...".into());
                    self.spawn_async_read(path, "clipboard-image".into(), Some(mime));
                }
                ActivityEvent::SaveAsPicked { uri, display_name } => {
                    log::info!("SAF Save As target: {display_name} -> {uri}");
                    self.status_banner = Some(format!("Save target ready: {display_name}"));
                }
                ActivityEvent::FontsSynced { count } => {
                    self.status_banner = Some(format!("Synced {count} user font(s)"));
                }
                ActivityEvent::McpServiceStopped => {
                    if let Some(srv) = self.mcp_server.take() {
                        srv.stop();
                    }
                    self.mcp_endpoint_label.clear();
                    self.status_banner = Some("MCP Server stopped".into());
                }
                _ => {}
            }
        }

        while let Ok(res) = self.asset_rx.try_recv() {
            match res {
                LoadedAsset::Document { display_name, bytes } => {
                    self.status_banner =
                        Some(format!("Opened {display_name} ({} KB)", bytes.len() / 1024));
                }
                LoadedAsset::Image {
                    display_name,
                    mime,
                    bytes,
                } => {
                    self.status_banner = Some(format!(
                        "Image ready: {display_name} ({mime}, {} KB)",
                        bytes.len() / 1024
                    ));
                }
                LoadedAsset::Err { display_name, error } => {
                    self.status_banner = Some(format!("Failed to load {display_name}: {error}"));
                }
            }
        }
    }

    fn drain_mcp_requests(&mut self) {
        let mut pending = Vec::new();
        if let Some(server) = &self.mcp_server {
            for _ in 0..8 {
                match server.rx.try_recv() {
                    Ok(req) => pending.push(req),
                    Err(_) => break,
                }
            }
        }
        for PendingMcpRequest { payload, reply_tx } in pending {
            let response = self.dispatch_mcp_jsonrpc(&payload);
            let _ = reply_tx.send(response);
        }
    }

    fn dispatch_mcp_jsonrpc(&mut self, req: &Value) -> Value {
        let id = req.get("id").cloned().unwrap_or(Value::Null);
        let method = req.get("method").and_then(Value::as_str).unwrap_or("");

        match method {
            "initialize" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": "2025-06-18",
                    "serverInfo": {
                        "name": "figcraft-android-mcp",
                        "version": env!("CARGO_PKG_VERSION")
                    },
                    "capabilities": {
                        "tools": {}
                    }
                }
            }),
            "tools/list" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "tools": [
                        {
                            "name": "run_command",
                            "description": "Execute any FigCraft engine command on the live Android tablet canvas.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "command": { "type": "string" },
                                    "params": { "type": "object" }
                                },
                                "required": ["command"]
                            }
                        }
                    ]
                }
            }),
            "tools/call" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "content": [{
                        "type": "text",
                        "text": "Wired to FigCraft Android shell (V6_hotfix3). Connect figcraft_mcp::Server to live engine."
                    }]
                }
            }),
            _ => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32601,
                    "message": format!("method not found: {method}")
                }
            }),
        }
    }

    fn toggle_mcp_lan(&mut self) {
        if let Some(old) = self.mcp_server.take() {
            old.stop();
        }
        self.mcp_lan_mode = !self.mcp_lan_mode;
        let mut cfg = McpServerConfig::default();
        if self.mcp_lan_mode {
            cfg.bind_addr = SocketAddr::from(([0, 0, 0, 0], 47800));
        }
        match EmbeddedMcpServer::start(cfg) {
            Ok(srv) => {
                let host = if self.mcp_lan_mode {
                    self.services
                        .get_local_ip()
                        .unwrap_or_else(|| "0.0.0.0".to_owned())
                } else {
                    "127.0.0.1".to_owned()
                };
                self.mcp_endpoint_label = format!("http://{host}:{}/mcp", srv.config.bind_addr.port());
                self.services
                    .set_mcp_foreground_service(true, &self.mcp_endpoint_label);
                self.status_banner = Some(format!("MCP: {}", self.mcp_endpoint_label));
                self.mcp_server = Some(srv);
            }
            Err(err) => {
                self.status_banner = Some(format!("MCP restart failed: {err}"));
            }
        }
    }
}

impl eframe::App for FigcraftAndroidShell {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.tablet_input.set_pixels_per_point(ctx.pixels_per_point());
        self.tablet_input.process_raw_input(raw_input);

        if let Some(text) = self.pending_paste_text.take() {
            if self.restore_focus_before_paste {
                if let Some(id) = self.last_focused_id {
                    ctx.memory_mut(|m| m.request_focus(id));
                }
                self.restore_focus_before_paste = false;
            }
            raw_input.events.push(egui::Event::Paste(text));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let ppp = ctx.pixels_per_point().max(1.0);
        self.tablet_input.set_pixels_per_point(ppp);

        if !self.restore_focus_before_paste && ctx.egui_wants_keyboard_input() {
            if let Some(focused) = ctx.memory(|m| m.focused()) {
                self.last_focused_id = Some(focused);
            }
        }

        self.drain_activity_events();
        self.drain_mcp_requests();

        let top_inset = self.tablet_input.safe_insets_px[0] / ppp;
        let right_inset = self.tablet_input.safe_insets_px[1] / ppp;
        let left_inset = self.tablet_input.safe_insets_px[3] / ppp;

        let top_btn_h = 34.0;
        let top_resp = egui::Panel::top("figcraft_android_topbar").show(ui, |ui| {
            ui.add_space(top_inset);
            egui::ScrollArea::horizontal()
                .id_salt("topbar_scroll")
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.add_space(left_inset);
                        if ui.add(egui::Button::new("Open").min_size(egui::vec2(52.0, top_btn_h))).clicked() {
                            let _ = self.services.request_open_file();
                        }
                        if ui.add(egui::Button::new("Image").min_size(egui::vec2(56.0, top_btn_h))).clicked() {
                            let _ = self.services.request_pick_image();
                        }
                        if ui.add(egui::Button::new("Save As").min_size(egui::vec2(64.0, top_btn_h))).clicked() {
                            let _ = self
                                .services
                                .request_save_as("design.figcraft", "application/x-figcraft");
                        }
                        if ui.add(egui::Button::new("Paste").min_size(egui::vec2(56.0, top_btn_h))).clicked() {
                            self.trigger_paste();
                        }
                        if ui.add(egui::Button::new("Fonts").min_size(egui::vec2(54.0, top_btn_h))).clicked() {
                            let _ = self.services.request_pick_fonts_dir();
                        }
                        ui.separator();
                        let zoom_label = format!("UI: {}%", (self.prefs.ui_zoom_factor * 100.0).round() as i32);
                        if ui.add(egui::Button::new(zoom_label).min_size(egui::vec2(72.0, top_btn_h))).clicked() {
                            self.cycle_ui_zoom(&ctx);
                        }
                        if ui
                            .add(
                                egui::Button::new("Full")
                                    .selected(self.prefs.immersive_mode)
                                    .min_size(egui::vec2(48.0, top_btn_h)),
                            )
                            .clicked()
                        {
                            self.toggle_immersive();
                        }
                        ui.separator();
                        let mcp_label = if self.mcp_lan_mode { "MCP: LAN" } else { "MCP: Local" };
                        if ui.add(egui::Button::new(mcp_label).min_size(egui::vec2(78.0, top_btn_h))).clicked() {
                            self.toggle_mcp_lan();
                        }
                        if let Some(srv) = &self.mcp_server {
                            if ui.add(egui::Button::new("Copy Token").min_size(egui::vec2(84.0, top_btn_h))).clicked() {
                                self.services.set_clipboard_text(&srv.config.bearer_token);
                                ctx.copy_text(srv.config.bearer_token.clone());
                                self.status_banner = Some("MCP Bearer token copied".into());
                            }
                        }
                        if ui
                            .add(
                                egui::Button::new("Pad")
                                    .selected(self.tablet_input.show_shortcut_pad)
                                    .min_size(egui::vec2(46.0, top_btn_h)),
                            )
                            .clicked()
                        {
                            self.tablet_input.show_shortcut_pad = !self.tablet_input.show_shortcut_pad;
                        }
                        if let Some(msg) = &self.status_banner {
                            ui.label(msg);
                        }
                        ui.add_space(right_inset);
                    });
                });
        });
        self.tablet_input.top_bar_bottom_pt = top_resp.response.rect.bottom();

        let central_resp = egui::CentralPanel::default().show(ui, |ui| {
            ui.centered_and_justified(|ui| {
                ui.label("FigCraft Android ARM64 Tablet Canvas V6_hotfix3");
            });
        });
        self.tablet_input.canvas_rect = Some(central_resp.response.rect);

        let pad_actions = self.tablet_input.show_shortcut_pad_ui(&ctx);
        if pad_actions.paste {
            self.trigger_paste();
        }
        if pad_actions.undo {
            log::info!("Shortcut Pad: Undo");
        }
        if pad_actions.redo {
            log::info!("Shortcut Pad: Redo");
        }
        if pad_actions.toggle_ime {
            let show = self.tablet_input.ime_height_px <= 1.0;
            self.services.set_soft_keyboard(show);
        }

        if self.tablet_input.needs_repaint()
            || self.pending_paste_text.is_some()
            || self.mcp_server.is_some()
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(40));
        }
    }
}
