//! Main `eframe::App` wrapper hosting the real `figcraft_ui_egui::FigApp` inside Android `GameActivity` (V8 · egui 0.37).

use crate::mcp_http::{EmbeddedMcpServer, McpServerConfig};
use crate::platform;
use crate::protocol::{ActivityEvent, StylusAction};
use crate::services::{AndroidServices, AndroidShellPrefs, ToolbarButtonId};
use crate::touch::TabletInputState;
use figcraft_engine::Session;
use figcraft_ui_egui::{FigApp, Services};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};

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

/// Bridges `figcraft_ui_egui::Services` to Android MediaStore, Clipboard, Browser, and Prefs.
struct AndroidFigServices {
    services: AndroidServices,
    ctx: egui::Context,
    pasted_png: Arc<Mutex<Option<Vec<u8>>>>,
    fig_prefs_path: PathBuf,
}

impl Services for AndroidFigServices {
    fn downloads(&self) -> bool {
        true
    }

    fn download(&mut self, name: &str, bytes: &[u8]) -> bool {
        let mime = if name.ends_with(".png") {
            "image/png"
        } else if name.ends_with(".svg") {
            "image/svg+xml"
        } else if name.ends_with(".pdf") {
            "application/pdf"
        } else if name.ends_with(".jpg") || name.ends_with(".jpeg") {
            "image/jpeg"
        } else {
            "application/octet-stream"
        };
        match self.services.save_via_mediastore("Exports", name, mime, bytes) {
            Ok(()) => true,
            Err(e) => {
                log::error!("Android download({name}) failed: {e}");
                false
            }
        }
    }

    fn clipboard_set_text(&mut self, s: &str) {
        self.services.set_clipboard_text(s);
        figcraft_ui_egui::egui_copy_text(&self.ctx, s);
    }

    fn clipboard_set_png(&mut self, w: u32, h: u32, rgba: &[u8]) {
        figcraft_ui_egui::egui_copy_png(&self.ctx, w, h, rgba);
    }

    fn clipboard_get_png(&mut self) -> Option<Vec<u8>> {
        self.pasted_png.lock().ok().and_then(|mut g| g.take())
    }

    fn open_url(&mut self, url: &str) {
        self.services.open_url(url);
    }

    fn open_url_here(&mut self, url: &str) {
        self.services.open_url(url);
    }

    fn save_prefs(&mut self, prefs: &figcraft_engine::Prefs) {
        if let Ok(json_str) = serde_json::to_string_pretty(prefs) {
            let _ = std::fs::write(&self.fig_prefs_path, json_str);
        }
    }

    fn system_language(&self) -> Option<String> {
        Some("ko".into())
    }
}

pub struct FigcraftAndroidShell {
    fig: FigApp,
    services: AndroidServices,
    prefs: AndroidShellPrefs,
    tablet_input: TabletInputState,
    mcp_server: Option<EmbeddedMcpServer>,
    mcp_lan_mode: bool,
    mcp_endpoint_label: String,
    status_banner: Option<String>,
    pending_paste_text: Option<String>,
    pasted_png: Arc<Mutex<Option<Vec<u8>>>>,
    last_focused_id: Option<egui::Id>,
    restore_focus_before_paste: bool,
    asset_tx: mpsc::Sender<LoadedAsset>,
    asset_rx: mpsc::Receiver<LoadedAsset>,
    ui_font_installed: bool,
    pending_ui_font_path: Option<PathBuf>,
    last_light_bars: Option<bool>,
    android_system_dark: Option<bool>,
    show_toolbar_editor: bool,
    pending_egui_events: Vec<egui::Event>,
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
        let services = AndroidServices::new(internal_dir.clone());
        let prefs = services.load_prefs();
        cc.egui_ctx.set_zoom_factor(prefs.ui_zoom_factor);
        if prefs.immersive_mode {
            services.set_immersive_mode(true);
        }

        // Index Android system fonts (/system/fonts) in background and repaint when ready
        let font_ctx = cc.egui_ctx.clone();
        std::thread::spawn(move || {
            figcraft_text::scan_system_fonts();
            font_ctx.request_repaint();
        });

        let fig_prefs_path = internal_dir.join("figcraft_prefs.json");
        let mut session = Session::new();
        if let Ok(raw) = std::fs::read_to_string(&fig_prefs_path) {
            if let Ok(loaded) = serde_json::from_str::<figcraft_engine::Prefs>(&raw) {
                session.prefs = loaded;
            }
        }
        figcraft_ui_egui::i18n::set(&session.prefs.language, Some("ko"));

        let pasted_png: Arc<Mutex<Option<Vec<u8>>>> = Arc::new(Mutex::new(None));
        let fig_services = AndroidFigServices {
            services: services.clone(),
            ctx: cc.egui_ctx.clone(),
            pasted_png: Arc::clone(&pasted_png),
            fig_prefs_path,
        };

        let mut fig = FigApp::new(session, Box::new(fig_services));
        fig.ui.home = false;
        fig.ui.title_bar = figcraft_ui_egui::TitleBar::Native;

        let (asset_tx, asset_rx) = mpsc::channel();
        let mut status_banner = None;
        let mut mcp_endpoint_label = String::new();

        let mcp_server = match EmbeddedMcpServer::start(McpServerConfig::default()) {
            Ok(srv) => {
                mcp_endpoint_label = format!("http://{}/mcp", srv.config.bind_addr);
                status_banner = Some(format!("MCP: {mcp_endpoint_label}"));
                Some(srv)
            }
            Err(err) => {
                log::warn!("MCP server disabled: {err}");
                None
            }
        };

        Self {
            fig,
            services,
            prefs,
            tablet_input: TabletInputState::default(),
            mcp_server,
            mcp_lan_mode: false,
            mcp_endpoint_label,
            status_banner,
            pending_paste_text: None,
            pasted_png,
            last_focused_id: None,
            restore_focus_before_paste: false,
            asset_tx,
            asset_rx,
            ui_font_installed: false,
            pending_ui_font_path: None,
            last_light_bars: None,
            android_system_dark: None,
            show_toolbar_editor: false,
            pending_egui_events: Vec::new(),
        }
    }

    fn install_ui_font_from_path(&mut self, ctx: &egui::Context, path: &std::path::Path, label: &str) -> bool {
        let Ok(bytes) = std::fs::read(path) else {
            return false;
        };
        if bytes.len() < 4096 {
            return false;
        }
        ctx.add_font(egui::FontInsert::new(
            format!("android_ui_font_{label}"),
            egui::FontData::from_owned(bytes),
            vec![
                egui::InsertFontFamily {
                    family: egui::FontFamily::Proportional,
                    priority: egui::FontPriority::Highest,
                },
                egui::InsertFontFamily {
                    family: egui::FontFamily::Monospace,
                    priority: egui::FontPriority::Lowest,
                },
            ],
        ));
        ctx.request_repaint();
        true
    }

    fn ensure_default_korean_ui_font(&mut self, ctx: &egui::Context) {
        if self.ui_font_installed {
            return;
        }
        self.ui_font_installed = true;
        let candidates = [
            (self.services.internal_dir.join("ui_font.custom"), "custom"),
            (self.services.internal_dir.join("bundled_ui_font.otf"), "pretendard"),
            (PathBuf::from("/system/fonts/NotoSansKR-Regular.otf"), "sys_notokr"),
            (PathBuf::from("/system/fonts/NotoSansCJK-Regular.ttc"), "sys_notocjk"),
            (PathBuf::from("/system/fonts/DroidSansFallback.ttf"), "sys_fallback"),
        ];
        for (path, label) in candidates {
            if self.install_ui_font_from_path(ctx, &path, label) {
                log::info!("Installed Android UI font: {}", path.display());
                break;
            }
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
        self.status_banner = Some(format!("화면 배율: {}%", (next * 100.0).round() as i32));
    }

    fn toggle_immersive(&mut self) {
        self.prefs.immersive_mode = !self.prefs.immersive_mode;
        self.services.set_immersive_mode(self.prefs.immersive_mode);
        self.services.save_prefs(&self.prefs);
        self.status_banner = Some(if self.prefs.immersive_mode {
            "전체화면(몰입모드) 켜짐".into()
        } else {
            "전체화면(몰입모드) 꺼짐".into()
        });
    }

    fn queue_key_shortcut(&mut self, key: egui::Key, ctrl: bool, shift: bool) {
        let modifiers = egui::Modifiers {
            alt: false,
            ctrl,
            shift,
            mac_cmd: false,
            command: ctrl,
        };
        self.pending_egui_events.push(egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers,
        });
        self.pending_egui_events.push(egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: false,
            repeat: false,
            modifiers,
        });
    }

    fn toggle_multi_select(&mut self) {
        self.tablet_input.multi_select_mode = !self.tablet_input.multi_select_mode;
        self.status_banner = Some(if self.tablet_input.multi_select_mode {
            "다중선택 켜짐 (도형·레이어를 차례로 탭해 누적 선택)".into()
        } else {
            "다중선택 꺼짐".into()
        });
    }

    fn render_toolbar_button(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, id: ToolbarButtonId, h: f32) {
        match id {
            ToolbarButtonId::Open => {
                if ui.add(egui::Button::new("열기").min_size(egui::vec2(46.0, h))).clicked() {
                    let _ = self.services.request_open_file();
                }
            }
            ToolbarButtonId::Image => {
                if ui.add(egui::Button::new("이미지").min_size(egui::vec2(52.0, h))).clicked() {
                    let _ = self.services.request_pick_image();
                }
            }
            ToolbarButtonId::Copy => {
                if ui.add(egui::Button::new("복사").min_size(egui::vec2(46.0, h))).clicked() {
                    self.pending_egui_events.push(egui::Event::Copy);
                    self.status_banner = Some("복사됨".into());
                }
            }
            ToolbarButtonId::Cut => {
                if ui.add(egui::Button::new("잘라내기").min_size(egui::vec2(62.0, h))).clicked() {
                    self.pending_egui_events.push(egui::Event::Cut);
                    self.status_banner = Some("잘라내기 완료".into());
                }
            }
            ToolbarButtonId::Paste => {
                if ui.add(egui::Button::new("붙여넣기").min_size(egui::vec2(62.0, h))).clicked() {
                    self.trigger_paste();
                }
            }
            ToolbarButtonId::Duplicate => {
                if ui.add(egui::Button::new("복제").min_size(egui::vec2(46.0, h))).clicked() {
                    self.queue_key_shortcut(egui::Key::D, true, false);
                    self.status_banner = Some("선택 개체 복제".into());
                }
            }
            ToolbarButtonId::Delete => {
                if ui.add(egui::Button::new("삭제").min_size(egui::vec2(46.0, h))).clicked() {
                    self.queue_key_shortcut(egui::Key::Delete, false, false);
                    self.status_banner = Some("선택 개체 삭제".into());
                }
            }
            ToolbarButtonId::MultiSelect => {
                if ui
                    .add(
                        egui::Button::new("다중선택")
                            .selected(self.tablet_input.multi_select_mode)
                            .min_size(egui::vec2(64.0, h)),
                    )
                    .clicked()
                {
                    self.toggle_multi_select();
                }
            }
            ToolbarButtonId::Undo => {
                if ui.add(egui::Button::new("실행취소").min_size(egui::vec2(62.0, h))).clicked() {
                    self.fig.run("edit.undo", json!({}));
                }
            }
            ToolbarButtonId::Redo => {
                if ui.add(egui::Button::new("다시실행").min_size(egui::vec2(62.0, h))).clicked() {
                    self.fig.run("edit.redo", json!({}));
                }
            }
            ToolbarButtonId::Group => {
                if ui.add(egui::Button::new("그룹").min_size(egui::vec2(46.0, h))).clicked() {
                    self.queue_key_shortcut(egui::Key::G, true, false);
                    self.status_banner = Some("그룹화".into());
                }
            }
            ToolbarButtonId::ZoomFit => {
                if ui.add(egui::Button::new("화면맞춤").min_size(egui::vec2(62.0, h))).clicked() {
                    self.queue_key_shortcut(egui::Key::Num1, false, true);
                }
            }
            ToolbarButtonId::Mouse => {
                if ui
                    .add(
                        egui::Button::new("마우스")
                            .selected(self.tablet_input.mouse_cursor_mode)
                            .min_size(egui::vec2(54.0, h)),
                    )
                    .clicked()
                {
                    self.toggle_mouse_mode();
                }
            }
            ToolbarButtonId::UiZoom => {
                let zoom_label = format!("배율: {}%", (self.prefs.ui_zoom_factor * 100.0).round() as i32);
                if ui.add(egui::Button::new(zoom_label).min_size(egui::vec2(76.0, h))).clicked() {
                    self.cycle_ui_zoom(ctx);
                }
            }
            ToolbarButtonId::Fullscreen => {
                if ui
                    .add(
                        egui::Button::new("전체화면")
                            .selected(self.prefs.immersive_mode)
                            .min_size(egui::vec2(62.0, h)),
                    )
                    .clicked()
                {
                    self.toggle_immersive();
                }
            }
            ToolbarButtonId::Fonts => {
                if ui.add(egui::Button::new("폰트").min_size(egui::vec2(46.0, h))).clicked() {
                    let _ = self.services.request_pick_fonts_dir();
                }
            }
            ToolbarButtonId::UiFont => {
                if ui.add(egui::Button::new("앱 폰트").min_size(egui::vec2(56.0, h))).clicked() {
                    let _ = self.services.request_pick_ui_font();
                }
            }
            ToolbarButtonId::Mcp => {
                let mcp_label = if self.mcp_lan_mode { "MCP: 공유" } else { "MCP: 로컬" };
                if ui.add(egui::Button::new(mcp_label).min_size(egui::vec2(76.0, h))).clicked() {
                    self.toggle_mcp_lan();
                }
            }
            ToolbarButtonId::CopyToken => {
                if let Some(srv) = &self.mcp_server {
                    if ui.add(egui::Button::new("토큰 복사").min_size(egui::vec2(70.0, h))).clicked() {
                        self.services.set_clipboard_text(&srv.config.bearer_token);
                        ctx.copy_text(srv.config.bearer_token.clone());
                        self.status_banner = Some("MCP 토큰이 복사되었습니다".into());
                    }
                }
            }
            ToolbarButtonId::Pad => {
                if ui
                    .add(
                        egui::Button::new("단축키")
                            .selected(self.tablet_input.show_shortcut_pad)
                            .min_size(egui::vec2(52.0, h)),
                    )
                    .clicked()
                {
                    self.tablet_input.show_shortcut_pad = !self.tablet_input.show_shortcut_pad;
                }
            }
        }
    }

    fn show_toolbar_editor_window(&mut self, ctx: &egui::Context) {
        if !self.show_toolbar_editor {
            return;
        }
        let mut open = self.show_toolbar_editor;
        let mut changed = false;

        egui::Window::new("상단바 버튼 편집")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(360.0)
            .show(ctx, |ui| {
                ui.label("표시 중인 버튼 (◀/▶ 순서 이동 · ✕ 숨기기)");
                ui.add_space(4.0);

                let mut move_left: Option<usize> = None;
                let mut move_right: Option<usize> = None;
                let mut remove_idx: Option<usize> = None;

                egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                    let len = self.prefs.toolbar_buttons.len();
                    for (idx, btn) in self.prefs.toolbar_buttons.iter().copied().enumerate() {
                        ui.horizontal(|ui| {
                            if ui.add_enabled(idx > 0, egui::Button::new("◀").min_size(egui::vec2(30.0, 28.0))).clicked() {
                                move_left = Some(idx);
                            }
                            if ui.add_enabled(idx + 1 < len, egui::Button::new("▶").min_size(egui::vec2(30.0, 28.0))).clicked() {
                                move_right = Some(idx);
                            }
                            ui.label(btn.display_name());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.add(egui::Button::new("✕").min_size(egui::vec2(28.0, 28.0))).clicked() {
                                    remove_idx = Some(idx);
                                }
                            });
                        });
                    }
                });

                if let Some(i) = move_left {
                    self.prefs.toolbar_buttons.swap(i - 1, i);
                    changed = true;
                } else if let Some(i) = move_right {
                    self.prefs.toolbar_buttons.swap(i, i + 1);
                    changed = true;
                } else if let Some(i) = remove_idx {
                    self.prefs.toolbar_buttons.remove(i);
                    changed = true;
                }

                ui.separator();
                ui.label("추가 가능한 버튼 (탭해서 상단바에 추가)");
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    for &btn in ToolbarButtonId::ALL {
                        if !self.prefs.toolbar_buttons.contains(&btn) {
                            if ui
                                .add(egui::Button::new(format!("+ {}", btn.display_name())).min_size(egui::vec2(64.0, 32.0)))
                                .clicked()
                            {
                                self.prefs.toolbar_buttons.push(btn);
                                changed = true;
                            }
                        }
                    }
                });

                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("기본값 복원").clicked() {
                        self.prefs.toolbar_buttons = ToolbarButtonId::default_bar();
                        changed = true;
                    }
                });
            });

        if changed {
            self.services.save_prefs(&self.prefs);
        }
        self.show_toolbar_editor = open;
    }

    fn toggle_mouse_mode(&mut self) {
        self.tablet_input.mouse_cursor_mode = !self.tablet_input.mouse_cursor_mode;
        self.status_banner = Some(if self.tablet_input.mouse_cursor_mode {
            "마우스 커서 켜짐 (스와이프=조준 · 탭=클릭 · 더블탭홀드=드래그)".into()
        } else {
            "마우스 커서 꺼짐 (직접 터치)".into()
        });
    }

    fn toggle_mcp_lan(&mut self) {
        let next_lan = !self.mcp_lan_mode;
        let bind_str = if next_lan { "0.0.0.0:47800" } else { "127.0.0.1:47800" };
        let Ok(bind_addr) = bind_str.parse::<SocketAddr>() else {
            return;
        };

        self.mcp_server = None;
        let cfg = McpServerConfig {
            bind_addr,
            ..McpServerConfig::default()
        };
        match EmbeddedMcpServer::start(cfg) {
            Ok(srv) => {
                self.mcp_lan_mode = next_lan;
                let display_host = if next_lan {
                    self.services
                        .get_local_ip()
                        .map(|ip| format!("{ip}:47800"))
                        .unwrap_or_else(|| "0.0.0.0:47800".into())
                } else {
                    "127.0.0.1:47800".into()
                };
                self.mcp_endpoint_label = format!("http://{display_host}/mcp");
                self.services
                    .set_mcp_foreground_service(next_lan, &self.mcp_endpoint_label);
                self.status_banner = Some(format!("MCP: {}", self.mcp_endpoint_label));
                self.mcp_server = Some(srv);
            }
            Err(err) => {
                self.status_banner = Some(format!("MCP bind failed: {err}"));
            }
        }
    }

    fn trigger_paste(&mut self) {
        self.restore_focus_before_paste = self.last_focused_id.is_some();
        if let Err(err) = self.services.request_paste_clipboard() {
            self.status_banner = Some(format!("Paste failed: {err}"));
        }
    }

    fn spawn_async_file_read(&self, staged_path: String, display_name: String) {
        let tx = self.asset_tx.clone();
        let _ = std::thread::Builder::new()
            .name("figcraft-doc-read".into())
            .spawn(move || {
                let res = std::fs::read(&staged_path);
                let _ = std::fs::remove_file(&staged_path);
                let msg = match res {
                    Ok(bytes) => LoadedAsset::Document { display_name, bytes },
                    Err(err) => LoadedAsset::Err {
                        display_name,
                        error: err.to_string(),
                    },
                };
                let _ = tx.send(msg);
            });
    }

    fn spawn_async_image_read(&self, staged_path: String, display_name: String, mime: String) {
        let tx = self.asset_tx.clone();
        let _ = std::thread::Builder::new()
            .name("figcraft-img-read".into())
            .spawn(move || {
                let res = std::fs::read(&staged_path);
                let _ = std::fs::remove_file(&staged_path);
                let msg = match res {
                    Ok(bytes) => LoadedAsset::Image {
                        display_name,
                        mime,
                        bytes,
                    },
                    Err(err) => LoadedAsset::Err {
                        display_name,
                        error: err.to_string(),
                    },
                };
                let _ = tx.send(msg);
            });
    }

    fn drain_activity_events(&mut self) {
        for ev in platform::poll_activity_events() {
            match ev {
                ActivityEvent::WindowInsets {
                    top,
                    right,
                    bottom,
                    left,
                } => {
                    self.tablet_input.safe_insets_px = [top, right, bottom, left];
                }
                ActivityEvent::ImeChanged { visible, height } => {
                    self.tablet_input.ime_height_px = if visible { height } else { 0.0 };
                }
                ActivityEvent::StylusSample {
                    action,
                    x,
                    y,
                    pressure,
                    tilt_x,
                    tilt_y,
                    is_eraser,
                } => {
                    let action_str = match action {
                        StylusAction::Down => "down",
                        StylusAction::Move => "move",
                        StylusAction::Up => "up",
                        StylusAction::Hover => "hover",
                    };
                    self.tablet_input
                        .on_stylus_event(action_str, x, y, pressure, tilt_x, tilt_y, is_eraser);
                }
                ActivityEvent::FileOpened { path, display_name } => {
                    self.status_banner = Some(format!("Loading {display_name}..."));
                    self.spawn_async_file_read(path, display_name);
                }
                ActivityEvent::ImagePicked {
                    path,
                    display_name,
                    mime,
                } => {
                    self.status_banner = Some(format!("Importing {display_name}..."));
                    self.spawn_async_image_read(path, display_name, mime);
                }
                ActivityEvent::ClipboardText(text) => {
                    self.pending_paste_text = Some(text);
                    self.status_banner = Some("클립보드 텍스트 붙여넣기 완료".into());
                }
                ActivityEvent::ClipboardImage { path, mime } => {
                    self.status_banner = Some("클립보드 이미지 붙여넣는 중...".into());
                    self.spawn_async_image_read(path, "clipboard-image.png".into(), mime);
                }
                ActivityEvent::SaveAsPicked { uri, display_name } => {
                    self.status_banner = Some(format!("Use FigCraft Export/Save -> Downloads ({display_name})"));
                    let _ = uri;
                }
                ActivityEvent::FontsSynced { count } => {
                    std::thread::spawn(|| {
                        figcraft_text::scan_system_fonts();
                    });
                    self.status_banner = Some(format!("Synced {count} user font(s)"));
                }
                ActivityEvent::UiFontPicked { path, display_name } => {
                    self.pending_ui_font_path = Some(PathBuf::from(path));
                    self.status_banner = Some(format!("UI Font: {display_name}"));
                }
                ActivityEvent::SystemTheme { dark } => {
                    self.android_system_dark = Some(dark);
                }
                ActivityEvent::McpServiceStopped => {
                    self.mcp_server = None;
                    self.mcp_lan_mode = false;
                    self.status_banner = Some("MCP stopped from notification".into());
                }
            }
        }

        while let Ok(loaded) = self.asset_rx.try_recv() {
            match loaded {
                LoadedAsset::Document { display_name, bytes } => {
                    let kb = bytes.len() / 1024;
                    let read: figcraft_ui_egui::ReadFuture = Box::pin(std::future::ready(Ok(bytes)));
                    self.fig.ui.pending_opens.push(figcraft_ui_egui::pending_open::PendingOpen {
                        label: display_name.clone(),
                        params: json!({ "name": display_name }),
                        command: "file.open",
                        target: None,
                        read: Some(read),
                    });
                    self.status_banner = Some(format!("Opened {display_name} ({kb} KB)"));
                }
                LoadedAsset::Image {
                    display_name,
                    mime,
                    bytes,
                } => {
                    let kb = bytes.len() / 1024;
                    if let Ok(mut slot) = self.pasted_png.lock() {
                        *slot = Some(bytes);
                    }
                    self.fig.run("edit.paste", json!({}));
                    self.status_banner = Some(format!("Imported {display_name} ({mime}, {kb} KB)"));
                }
                LoadedAsset::Err {
                    display_name,
                    error,
                } => {
                    self.status_banner = Some(format!("Failed to load {display_name}: {error}"));
                }
            }
        }
    }

    fn drain_mcp_requests(&mut self) {
        let mut pending = Vec::new();
        if let Some(srv) = &self.mcp_server {
            while let Ok(req) = srv.rx.try_recv() {
                pending.push(req);
            }
        }
        for req in pending {
            let reply = self.handle_mcp_jsonrpc(req.payload);
            let _ = req.reply_tx.send(reply);
        }
    }

    fn handle_mcp_jsonrpc(&mut self, req: Value) -> Value {
        let id = req.get("id").cloned().unwrap_or(Value::Null);
        let method = req.get("method").and_then(Value::as_str).unwrap_or("");
        match method {
            "initialize" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": "2025-03-26",
                    "serverInfo": { "name": "figcraft-android-mcp", "version": "0.18.1" },
                    "capabilities": { "tools": {} }
                }
            }),
            "tools/list" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "tools": [
                        { "name": "engine.execute", "description": "Execute a FigCraft engine command on the live tablet document" },
                        { "name": "figcraft.status", "description": "Return Android tablet shell & document status" }
                    ]
                }
            }),
            "tools/call" => {
                let name = req
                    .pointer("/params/name")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let args = req
                    .pointer("/params/arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                if name == "engine.execute" {
                    let cmd = args.get("command").and_then(Value::as_str).unwrap_or("");
                    let cmd_params = args.get("params").cloned().unwrap_or_else(|| json!({}));
                    match self.fig.session.execute(cmd, &cmd_params) {
                        Ok(out) => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "content": [{ "type": "text", "text": format!("{out:?}") }]
                            }
                        }),
                        Err(err) => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32000, "message": format!("{err}") }
                        }),
                    }
                } else {
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "content": [{
                                "type": "text",
                                "text": format!(
                                    "FigCraft Android V8 active (docs={}, endpoint={}, zoom={}%, mouse={})",
                                    self.fig.session.docs.len(),
                                    self.mcp_endpoint_label,
                                    (self.prefs.ui_zoom_factor * 100.0).round() as i32,
                                    self.tablet_input.mouse_cursor_mode
                                )
                            }]
                        }
                    })
                }
            }
            _ => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": format!("Method not found: {method}") }
            }),
        }
    }
}

impl eframe::App for FigcraftAndroidShell {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        figcraft_ui_egui::windows::guard(&mut self.fig, ctx, |fig| fig.background_tick(ctx));
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.tablet_input.set_pixels_per_point(ctx.pixels_per_point());
        self.tablet_input.screen_rect = ctx.input(|i| i.viewport_rect());
        if let Some(dark) = self.android_system_dark {
            raw_input.system_theme = Some(if dark { egui::Theme::Dark } else { egui::Theme::Light });
        }
        self.tablet_input.process_raw_input(raw_input);

        let control_events = figcraft_ui_egui::windows::take_input(&mut self.fig, raw_input.viewport_id);
        raw_input.events.extend(control_events);

        if !self.pending_egui_events.is_empty() {
            raw_input.events.extend(self.pending_egui_events.drain(..));
        }

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
        self.tablet_input.screen_rect = ctx.input(|i| i.viewport_rect());

        if !self.restore_focus_before_paste && ctx.egui_wants_keyboard_input() {
            if let Some(focused) = ctx.memory(|m| m.focused()) {
                self.last_focused_id = Some(focused);
            }
        }

        self.drain_activity_events();
        self.drain_mcp_requests();

        // Sync Android status/nav bar icon color with FigCraft light/dark theme
        let is_light_theme = !ctx.style().visuals.dark_mode;
        if self.last_light_bars != Some(is_light_theme) {
            self.last_light_bars = Some(is_light_theme);
            self.services.set_light_system_bars(is_light_theme);
        }

        let top_inset = self.tablet_input.safe_insets_px[0] / ppp;
        let right_inset = self.tablet_input.safe_insets_px[1] / ppp;
        let bottom_inset = self.tablet_input.safe_insets_px[2] / ppp;
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
                        let buttons = self.prefs.toolbar_buttons.clone();
                        for btn_id in buttons {
                            self.render_toolbar_button(ui, &ctx, btn_id, top_btn_h);
                        }
                        if ui
                            .add(
                                egui::Button::new("+")
                                    .selected(self.show_toolbar_editor)
                                    .min_size(egui::vec2(36.0, top_btn_h)),
                            )
                            .clicked()
                        {
                            self.show_toolbar_editor = !self.show_toolbar_editor;
                        }
                        if let Some(msg) = &self.status_banner {
                            ui.label(msg);
                        }
                        ui.add_space(right_inset);
                    });
                });
        });
        self.tablet_input.top_bar_bottom_pt = top_resp.response.rect.bottom();

        // Reserve bottom system taskbar height when NOT in immersive fullscreen
        // so FigCraft's floating bottom tool dock stays cleanly ABOVE the Android taskbar!
        if !self.prefs.immersive_mode && bottom_inset > 1.0 {
            egui::Panel::bottom("figcraft_android_bottom_inset")
                .frame(egui::Frame::NONE)
                .show_separator_line(false)
                .show(ui, |ui| {
                    ui.allocate_space(egui::vec2(ui.available_width(), bottom_inset));
                });
        }

        self.tablet_input.canvas_rect = Some(ui.available_rect_before_wrap());

        // Render the real FigCraft UI (toolbars, Layers, Design/Inspect panels, Canvas)
        figcraft_ui_egui::windows::guard(&mut self.fig, &ctx, |fig| fig.ui(ui));

        // Install bundled/custom Korean UI font AFTER FigApp initializes its own icon fonts
        self.ensure_default_korean_ui_font(&ctx);
        if let Some(font_path) = self.pending_ui_font_path.take() {
            let _ = self.install_ui_font_from_path(&ctx, &font_path, "picked");
        }

        // Enforce Android one-touch UI Scale AFTER FigApp::ui() so FigApp prefs do not reset it to 100%
        if (self.prefs.ui_zoom_factor - 1.0).abs() > 0.01
            && (ctx.zoom_factor() - self.prefs.ui_zoom_factor).abs() > 0.01
        {
            ctx.set_zoom_factor(self.prefs.ui_zoom_factor);
        }

        // Floating tablet Shortcut Pad + Left-Top Offset Mouse Cursor overlay
        let pad_actions = self.tablet_input.show_shortcut_pad_ui(&ctx);
        if pad_actions.paste {
            self.trigger_paste();
        }
        if pad_actions.undo {
            self.fig.run("edit.undo", json!({}));
        }
        if pad_actions.redo {
            self.fig.run("edit.redo", json!({}));
        }
        if pad_actions.toggle_mouse {
            self.toggle_mouse_mode();
        }
        if pad_actions.toggle_ime {
            let show = self.tablet_input.ime_height_px <= 1.0;
            self.services.set_soft_keyboard(show);
        }

        self.show_toolbar_editor_window(&ctx);
        self.tablet_input.paint_virtual_cursor(&ctx);

        if self.tablet_input.needs_repaint()
            || self.pending_paste_text.is_some()
            || self.mcp_server.is_some()
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(40));
        }
    }
}
