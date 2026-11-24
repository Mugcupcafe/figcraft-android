//! Main `eframe::App` wrapper hosting the real `figcraft_ui_egui::FigApp` inside Android `GameActivity` (V8 · egui 0.37).

use crate::agent_bubble::AgentBubbleState;
use crate::mcp_http::{self, EmbeddedMcpServer, McpServerConfig};
use crate::ngrok_tunnel::{NgrokEvent, NgrokTunnelHandle};
use crate::platform;
use crate::protocol::{ActivityEvent, StylusAction};
use crate::services::{AndroidServices, AndroidShellPrefs, ToolbarButtonId, TunnelProvider};
use crate::touch::TabletInputState;
use figcraft_engine::branches::{needs_copy, recovery_copy, stamp};
use figcraft_engine::Session;
use figcraft_ui_egui::{FigApp, Services};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FontCategory {
    All,
    Variable,
    Korean,
    Sans,
    Serif,
    Mono,
}

impl FontCategory {
    const ALL: &[Self] = &[
        Self::All,
        Self::Variable,
        Self::Korean,
        Self::Sans,
        Self::Serif,
        Self::Mono,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::All => "전체",
            Self::Variable => "가변글꼴(VF)",
            Self::Korean => "한글",
            Self::Sans => "고딕(Sans)",
            Self::Serif => "명조(Serif)",
            Self::Mono => "고정폭(Mono)",
        }
    }
}

#[derive(Debug, Clone)]
struct FontCatalogEntry {
    family: String,
    display_name: String,
    is_variable: bool,
    is_korean: bool,
    style: FontCategory,
    source_tag: &'static str,
}

#[derive(Debug, Clone, serde::Deserialize, Default)]
struct DriveCanvasItem {
    #[serde(default)]
    file_id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    modified_time: String,
    #[serde(default)]
    local_path: String,
    #[serde(default)]
    thumb_path: String,
    #[serde(default)]
    synced: bool,
}

enum LoadedAsset {
    Document {
        display_name: String,
        bytes: Vec<u8>,
    },
    Image {
        staged_path: String,
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
        self.pasted_png.lock().ok().and_then(|g| g.clone())
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
    drive_window_open: bool,
    drive_window_anchor: Option<egui::Pos2>,
    drive_window_open_seq: u64,
    toolbar_editor_anchor: Option<egui::Pos2>,
    toolbar_editor_open_seq: u64,
    drive_signed_in: bool,
    drive_email: String,
    drive_status: String,
    drive_items: Vec<DriveCanvasItem>,
    last_focused_id: Option<egui::Id>,
    restore_focus_before_paste: bool,
    asset_tx: mpsc::Sender<LoadedAsset>,
    asset_rx: mpsc::Receiver<LoadedAsset>,
    ui_font_frame_count: u32,
    pending_ui_font_path: Option<PathBuf>,
    cached_ui_font: Option<(String, Vec<u8>)>,
    font_browser_open: bool,
    font_browser_anchor: Option<egui::Pos2>,
    font_browser_open_seq: u64,
    font_filter: FontCategory,
    font_search: String,
    font_catalog: Vec<FontCatalogEntry>,
    java_files_dir: Option<PathBuf>,
    last_light_bars: Option<bool>,
    android_system_dark: Option<bool>,
    show_toolbar_editor: bool,
    show_mouse_settings: bool,
    mouse_btn_press_start: Option<std::time::Instant>,
    mouse_btn_long_fired: bool,
    pending_egui_events: Vec<egui::Event>,
    last_wants_keyboard: bool,
    ime_composing: bool,
    game_ime_active: bool,
    game_ime_prev_committed: String,
    game_ime_last_composing: String,
    last_recovery_stamps: Vec<(u64, Vec<u64>)>,
    last_recovery_save: std::time::Instant,
    agent_bubble: AgentBubbleState,
    mcp_window_open: bool,
    mcp_window_anchor: Option<egui::Pos2>,
    mcp_window_open_seq: u64,
    ngrok_tunnel: Option<NgrokTunnelHandle>,
    tunnel_state: String,
    tunnel_url: String,
    tunnel_msg: String,
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
        let mut prefs = services.load_prefs();
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
        let recovery_files = services.read_recovery_snapshots();
        let mut session = Session::empty();
        let live_ctx = cc.egui_ctx.clone();
        session.live.install(Box::new(figcraft_collab_net::NetLink::new(Arc::new(move || {
            live_ctx.request_repaint();
        }))));
        let mut recovered_count = 0usize;
        for bytes in &recovery_files {
            let b64 = crate::services::encode_base64(bytes);
            if session.execute("file.recover", &json!({ "data": b64 })).is_ok() {
                recovered_count += 1;
            }
        }
        if session.docs.is_empty() {
            let _ = session.execute("file.new", &json!({}));
        }
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
        if recovered_count > 0 {
            fig.note_recovered(recovered_count);
        }
        fig.ui.home = false;
        fig.ui.title_bar = figcraft_ui_egui::TitleBar::Native;

        let (asset_tx, asset_rx) = mpsc::channel();
        let mut status_banner = None;
        let mut mcp_endpoint_label = String::new();

        if prefs.mcp_bearer_token.trim().is_empty() {
            prefs.mcp_bearer_token = mcp_http::generate_session_token();
            services.save_prefs(&prefs);
        }
        let initial_mcp_cfg = McpServerConfig::with_token(
            "127.0.0.1:47800".parse().expect("valid loopback addr"),
            prefs.mcp_bearer_token.clone(),
        );
        let mcp_server = match EmbeddedMcpServer::start(initial_mcp_cfg) {
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

        let mut tablet_input = TabletInputState::default();
        tablet_input.mouse_cursor_scale = prefs.mouse_cursor_scale;
        tablet_input.mouse_grip_distance = prefs.mouse_grip_distance;
        tablet_input.mouse_grip_offset_x = prefs.mouse_grip_offset_x;
        tablet_input.mouse_grip_offset_y = prefs.mouse_grip_offset_y;
        tablet_input.mouse_grip_opacity = prefs.mouse_grip_opacity;
        tablet_input.mouse_border_width = prefs.mouse_border_width;
        tablet_input.show_shortcut_pad = prefs.show_shortcut_pad;

        Self {
            fig,
            services,
            prefs,
            tablet_input,
            mcp_server,
            mcp_lan_mode: false,
            mcp_endpoint_label,
            status_banner,
            pending_paste_text: None,
            pasted_png,
            drive_window_open: false,
            drive_window_anchor: None,
            drive_window_open_seq: 0,
            toolbar_editor_anchor: None,
            toolbar_editor_open_seq: 0,
            drive_signed_in: false,
            drive_email: String::new(),
            drive_status: String::new(),
            drive_items: Vec::new(),
            last_focused_id: None,
            restore_focus_before_paste: false,
            asset_tx,
            asset_rx,
            ui_font_frame_count: 0,
            pending_ui_font_path: None,
            cached_ui_font: None,
            font_browser_open: false,
            font_browser_anchor: None,
            font_browser_open_seq: 0,
            font_filter: FontCategory::All,
            font_search: String::new(),
            font_catalog: Vec::new(),
            java_files_dir: None,
            last_light_bars: None,
            android_system_dark: None,
            show_toolbar_editor: false,
            show_mouse_settings: false,
            mouse_btn_press_start: None,
            mouse_btn_long_fired: false,
            pending_egui_events: Vec::new(),
            last_wants_keyboard: false,
            ime_composing: false,
            game_ime_active: false,
            game_ime_prev_committed: " ".to_owned(),
            game_ime_last_composing: String::new(),
            last_recovery_stamps: Vec::new(),
            last_recovery_save: std::time::Instant::now(),
            agent_bubble: AgentBubbleState::default(),
            mcp_window_open: false,
            mcp_window_anchor: None,
            mcp_window_open_seq: 0,
            ngrok_tunnel: None,
            tunnel_state: "stopped".into(),
            tunnel_url: String::new(),
            tunnel_msg: String::new(),
        }
    }

    fn install_ui_font_from_path(&mut self, ctx: &egui::Context, path: &std::path::Path, label: &str) -> bool {
        let cache_key = path.to_string_lossy().into_owned();
        let bytes = if let Some((k, b)) = &self.cached_ui_font {
            if k == &cache_key {
                b.clone()
            } else {
                let Ok(read_b) = std::fs::read(path) else { return false; };
                if read_b.len() < 4096 { return false; }
                self.cached_ui_font = Some((cache_key, read_b.clone()));
                read_b
            }
        } else {
            let Ok(read_b) = std::fs::read(path) else { return false; };
            if read_b.len() < 4096 { return false; }
            self.cached_ui_font = Some((cache_key, read_b.clone()));
            read_b
        };
        let existing_families: Vec<egui::FontFamily> = ctx.fonts(|f| f.families());
        let mut targets: Vec<egui::InsertFontFamily> = Vec::new();
        for family in existing_families {
            let skip = match &family {
                egui::FontFamily::Name(name) => {
                    let lower = name.to_lowercase();
                    lower.contains("icon")
                        || lower.contains("emoji")
                        || lower.contains("symbol")
                        || lower.contains("phosphor")
                        || lower.contains("lucide")
                        || lower.contains("material")
                }
                _ => false,
            };
            if !skip {
                targets.push(egui::InsertFontFamily {
                    family,
                    priority: egui::FontPriority::Highest,
                });
            }
        }
        if targets.is_empty() {
            targets.push(egui::InsertFontFamily {
                family: egui::FontFamily::Proportional,
                priority: egui::FontPriority::Highest,
            });
            targets.push(egui::InsertFontFamily {
                family: egui::FontFamily::Monospace,
                priority: egui::FontPriority::Highest,
            });
        }
        ctx.add_font(egui::FontInsert::new(
            &format!("android_ui_font_{label}"),
            egui::FontData::from_owned(bytes),
            targets,
        ));
        ctx.request_repaint();
        true
    }

    fn ensure_default_korean_ui_font(&mut self, ctx: &egui::Context) {
        self.ui_font_frame_count = self.ui_font_frame_count.saturating_add(1);
        // Re-apply on frames 1, 10, 40, and 90 so FigApp startup or background font scan cannot wipe out our custom/bundled UI font
        if !matches!(self.ui_font_frame_count, 1 | 10 | 40 | 90) {
            return;
        }
        let base_dir = self
            .java_files_dir
            .clone()
            .unwrap_or_else(|| self.services.internal_dir.clone());
        let candidates = [
            (base_dir.join("ui_font.custom"), "custom"),
            (self.services.internal_dir.join("ui_font.custom"), "custom_alt"),
            (base_dir.join("bundled_ui_font.otf"), "pretendard"),
            (self.services.internal_dir.join("bundled_ui_font.otf"), "pretendard_alt"),
            (PathBuf::from("/system/fonts/NotoSansKR-Regular.otf"), "sys_notokr"),
            (PathBuf::from("/system/fonts/NotoSansCJK-Regular.ttc"), "sys_notocjk"),
            (PathBuf::from("/system/fonts/DroidSansFallback.ttf"), "sys_fallback"),
        ];
        for (path, label) in candidates {
            if self.install_ui_font_from_path(ctx, &path, label) {
                log::info!("Installed Android UI font (frame {}): {}", self.ui_font_frame_count, path.display());
                break;
            }
        }
    }

    fn current_recovery_stamps(&self) -> Vec<(u64, Vec<u64>)> {
        self.fig
            .session
            .docs
            .iter()
            .filter(|d| needs_copy(&self.fig.session, d))
            .map(|d| (d.uid, stamp(&self.fig.session, d)))
            .collect()
    }

    fn save_recovery_snapshots(&mut self, force: bool) {
        let stamps = self.current_recovery_stamps();
        if !force && stamps == self.last_recovery_stamps {
            return;
        }
        self.last_recovery_stamps = stamps;
        self.last_recovery_save = std::time::Instant::now();
        if self.prefs.drive_auto_sync && self.drive_signed_in {
            self.upload_active_canvas_to_drive();
        }

        let copies: Vec<_> = self
            .fig
            .session
            .docs
            .iter()
            .filter(|d| needs_copy(&self.fig.session, d))
            .map(|d| recovery_copy(&self.fig.session, d))
            .collect();
        let recovery_dir = self.services.recovery_dir.clone();

        let _ = std::thread::Builder::new()
            .name("figcraft-autosave".into())
            .spawn(move || {
                let encoded: Vec<Vec<u8>> = copies
                    .into_iter()
                    .filter_map(|(doc, versions, branches)| {
                        figcraft_format::save_bytes_with_history(&doc, None, &versions, &branches).ok()
                    })
                    .collect();
                AndroidServices::replace_recovery_snapshots(&recovery_dir, &encoded);
            });
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
                let resp = ui.add(
                    egui::Button::new("마우스")
                        .selected(self.tablet_input.mouse_cursor_mode)
                        .min_size(egui::vec2(54.0, h)),
                );
                if resp.is_pointer_button_down_on() {
                    let started = *self.mouse_btn_press_start.get_or_insert_with(std::time::Instant::now);
                    if !self.mouse_btn_long_fired && started.elapsed() >= std::time::Duration::from_millis(450) {
                        self.mouse_btn_long_fired = true;
                        self.show_mouse_settings = true;
                    }
                    let ms = if self.game_ime_active { 16 } else { 40 };
            ctx.request_repaint_after(std::time::Duration::from_millis(ms));
                } else {
                    if resp.clicked() && !self.mouse_btn_long_fired {
                        self.toggle_mouse_mode();
                    }
                    self.mouse_btn_press_start = None;
                    self.mouse_btn_long_fired = false;
                }
            }
            ToolbarButtonId::UiZoom => {}
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
                let resp = ui.add(
                    egui::Button::new("폰트")
                        .selected(self.font_browser_open)
                        .min_size(egui::vec2(46.0, h)),
                );
                if resp.clicked() {
                    self.font_browser_open = !self.font_browser_open;
                    if self.font_browser_open {
                        self.font_browser_anchor = Some(egui::pos2(resp.rect.left(), resp.rect.bottom() + 6.0));
                        self.font_browser_open_seq = self.font_browser_open_seq.wrapping_add(1);
                        self.refresh_font_catalog();
                    }
                }
            }
            ToolbarButtonId::UiFont => {
                if ui.add(egui::Button::new("앱 폰트").min_size(egui::vec2(56.0, h))).clicked() {
                    let _ = self.services.request_pick_ui_font();
                }
            }
            ToolbarButtonId::TextInput => {
                if ui.add(egui::Button::new("텍스트입력").min_size(egui::vec2(76.0, h))).clicked() {
                    self.restore_focus_before_paste = true;
                    self.services.prompt_text_input("");
                }
            }
            ToolbarButtonId::RenameCanvas => {
                if ui.add(egui::Button::new("이름변경").min_size(egui::vec2(68.0, h))).clicked() {
                    let cur = self
                        .fig
                        .session
                        .docs
                        .get(self.fig.session.active)
                        .map(|d| d.doc.name.clone())
                        .unwrap_or_else(|| "Untitled".into());
                    self.services.prompt_rename_canvas(&cur);
                }
            }
            ToolbarButtonId::Drive => {
                let label = if self.drive_signed_in { "드라이브 ●" } else { "드라이브" };
                let resp = ui.add(egui::Button::new(label).min_size(egui::vec2(72.0, h)));
                if resp.clicked() {
                    self.drive_window_open = !self.drive_window_open;
                    if self.drive_window_open {
                        self.drive_window_anchor = Some(egui::pos2(resp.rect.left(), resp.rect.bottom() + 6.0));
                        self.drive_window_open_seq = self.drive_window_open_seq.wrapping_add(1);
                        if self.drive_signed_in {
                            self.services.drive_sync_now();
                        }
                    }
                }
            }
            ToolbarButtonId::Mcp => {
                let mcp_label = if self.tunnel_state == "connected" {
                    "MCP ●"
                } else if self.tunnel_state == "connecting" {
                    "MCP …"
                } else if self.mcp_lan_mode {
                    "MCP: 공유"
                } else {
                    "MCP: 로컬"
                };
                let resp = ui.add(
                    egui::Button::new(mcp_label)
                        .selected(self.mcp_window_open || self.tunnel_state == "connected")
                        .min_size(egui::vec2(76.0, h)),
                );
                if resp.clicked() {
                    self.mcp_window_open = !self.mcp_window_open;
                    if self.mcp_window_open {
                        self.mcp_window_anchor = Some(egui::pos2(resp.rect.left(), resp.rect.bottom() + 6.0));
                        self.mcp_window_open_seq = self.mcp_window_open_seq.wrapping_add(1);
                    }
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
                    self.prefs.show_shortcut_pad = self.tablet_input.show_shortcut_pad;
                    self.services.save_prefs(&self.prefs);
                }
            }
            ToolbarButtonId::AiAgent => {
                if ui
                    .add(
                        egui::Button::new("AI 에이전트")
                            .selected(self.prefs.ai_bubble_visible)
                            .min_size(egui::vec2(78.0, h)),
                    )
                    .clicked()
                {
                    self.prefs.ai_bubble_visible = !self.prefs.ai_bubble_visible;
                    self.services.save_prefs(&self.prefs);
                }
            }
        }
    }

    fn clamped_popup_pos(ctx: &egui::Context, anchor: Option<egui::Pos2>, win_w: f32, win_h: f32) -> egui::Pos2 {
        let screen = ctx.input(|i| i.viewport_rect());
        let raw = anchor.unwrap_or_else(|| egui::pos2(screen.center().x - win_w * 0.5, screen.top() + 64.0));
        let min_x = screen.left() + 8.0;
        let max_x = (screen.right() - win_w - 8.0).max(min_x);
        let min_y = screen.top() + 8.0;
        let max_y = (screen.bottom() - win_h - 8.0).max(min_y);
        egui::pos2(raw.x.clamp(min_x, max_x), raw.y.clamp(min_y, max_y))
    }

    fn poll_ngrok_tunnel(&mut self) {
        let mut events = Vec::new();
        if let Some(handle) = &self.ngrok_tunnel {
            while let Ok(ev) = handle.rx.try_recv() {
                events.push(ev);
            }
        }
        for ev in events {
            match ev {
                NgrokEvent::Connecting(msg) => {
                    self.tunnel_state = "connecting".into();
                    self.tunnel_msg = msg;
                }
                NgrokEvent::Connected { mcp_url } => {
                    self.tunnel_state = "connected".into();
                    self.tunnel_url = mcp_url;
                    self.tunnel_msg = "ngrok 연결 완료".into();
                }
                NgrokEvent::Error(msg) => {
                    self.tunnel_state = "error".into();
                    self.tunnel_url.clear();
                    self.tunnel_msg = msg;
                    self.ngrok_tunnel = None;
                }
                NgrokEvent::Stopped => {
                    self.tunnel_state = "stopped".into();
                    self.tunnel_url.clear();
                    self.tunnel_msg = "ngrok 터널 중지됨".into();
                    self.ngrok_tunnel = None;
                }
            }
        }
    }

    fn regenerate_mcp_token(&mut self) {
        let new_tok = mcp_http::generate_session_token();
        self.prefs.mcp_bearer_token = new_tok.clone();
        self.services.save_prefs(&self.prefs);
        let bind_str = if self.mcp_lan_mode { "0.0.0.0:47800" } else { "127.0.0.1:47800" };
        if let Ok(bind_addr) = bind_str.parse::<SocketAddr>() {
            self.mcp_server = None;
            let cfg = McpServerConfig::with_token(bind_addr, new_tok);
            if let Ok(srv) = EmbeddedMcpServer::start(cfg) {
                self.mcp_server = Some(srv);
                self.status_banner = Some("MCP 토큰이 새로 발급되었습니다".into());
            }
        }
    }

    fn stop_active_tunnel(&mut self) {
        if let Some(mut ng) = self.ngrok_tunnel.take() {
            ng.stop();
        }
        self.services.tunnel_stop_cf();
        self.tunnel_state = "stopped".into();
        self.tunnel_url.clear();
        self.tunnel_msg = "터널이 중지되었습니다".into();
    }

    fn start_selected_tunnel(&mut self) {
        if let Some(mut ng) = self.ngrok_tunnel.take() {
            ng.stop();
        }
        self.services.tunnel_stop_cf();
        let port = self
            .mcp_server
            .as_ref()
            .map(|s| s.config.bind_addr.port())
            .unwrap_or(47800);
        match self.prefs.tunnel_provider {
            TunnelProvider::CloudflareQuick => {
                self.tunnel_state = "connecting".into();
                self.tunnel_url.clear();
                self.tunnel_msg = "Cloudflare 퀵터널 시작 중...".into();
                self.services.tunnel_start_cf_quick(port);
            }
            TunnelProvider::CloudflareNamed => {
                self.tunnel_state = "connecting".into();
                self.tunnel_url.clear();
                self.tunnel_msg = "Cloudflare 고정터널 시작 중...".into();
                self.services.tunnel_start_cf_named(
                    &self.prefs.cf_tunnel_token,
                    &self.prefs.cf_tunnel_domain,
                );
            }
            TunnelProvider::Ngrok => {
                self.tunnel_state = "connecting".into();
                self.tunnel_url.clear();
                self.tunnel_msg = "ngrok 연결 중...".into();
                self.ngrok_tunnel = Some(NgrokTunnelHandle::start(
                    self.prefs.ngrok_authtoken.clone(),
                    self.prefs.ngrok_domain.clone(),
                    port,
                ));
            }
        }
    }

    fn show_mcp_tunnel_window(&mut self, ctx: &egui::Context) {
        if !self.mcp_window_open {
            return;
        }
        let mut open = self.mcp_window_open;
        let win_w = 340.0;
        let pos = Self::clamped_popup_pos(ctx, self.mcp_window_anchor, win_w, 430.0);
        let bearer_token = self
            .mcp_server
            .as_ref()
            .map(|s| s.config.bearer_token.clone())
            .unwrap_or_default();

        egui::Window::new("MCP · HTTPS 터널")
            .order(egui::Order::Foreground)
            .id(egui::Id::unique(("mcp_tunnel_win", self.mcp_window_open_seq)))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .fixed_pos(pos)
            .fixed_size(egui::vec2(win_w, 0.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let local_lbl = if self.mcp_lan_mode { "Wi-Fi 공유 켜짐" } else { "로컬 전용 (127.0.0.1)" };
                    ui.strong(local_lbl);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let toggle_lbl = if self.mcp_lan_mode { "로컬로 전환" } else { "Wi-Fi 공유" };
                        if ui.button(toggle_lbl).clicked() {
                            self.toggle_mcp_lan();
                        }
                    });
                });
                ui.horizontal(|ui| {
                    ui.small(&self.mcp_endpoint_label);
                    if !bearer_token.is_empty() && ui.small_button("토큰 복사").clicked() {
                        self.services.set_clipboard_text(&bearer_token);
                        ctx.copy_text(bearer_token.clone());
                        self.status_banner = Some("고정 MCP 토큰 복사됨".into());
                    }
                    if ui.small_button("재발급").clicked() {
                        self.regenerate_mcp_token();
                    }
                });
                ui.weak("토큰은 앱 재실행·Wi-Fi 전환 후에도 고정 유지됩니다.");

                ui.add_space(6.0);
                ui.separator();
                ui.add_space(4.0);

                ui.strong("공개 HTTPS 터널 방식 선택");
                let mut prefs_dirty = false;
                if ui
                    .radio_value(
                        &mut self.prefs.tunnel_provider,
                        TunnelProvider::CloudflareQuick,
                        "Cloudflare 퀵터널 (무료 · 무로그인 · 임시 주소)",
                    )
                    .changed()
                {
                    prefs_dirty = true;
                }
                if ui
                    .radio_value(
                        &mut self.prefs.tunnel_provider,
                        TunnelProvider::CloudflareNamed,
                        "Cloudflare 고정터널 (토큰 · 본인 도메인)",
                    )
                    .changed()
                {
                    prefs_dirty = true;
                }
                if ui
                    .radio_value(
                        &mut self.prefs.tunnel_provider,
                        TunnelProvider::Ngrok,
                        "ngrok (무료 계정 · 0원 고정 도메인)",
                    )
                    .changed()
                {
                    prefs_dirty = true;
                }

                ui.add_space(6.0);
                match self.prefs.tunnel_provider {
                    TunnelProvider::CloudflareQuick => {
                        ui.weak("별도 가입 없이 버튼 한 번으로 https://xxxx.trycloudflare.com/mcp 주소를 발급합니다.");
                    }
                    TunnelProvider::CloudflareNamed => {
                        ui.small("Cloudflare Tunnel Token");
                        if ui
                            .add(
                                egui::TextEdit::singleline(&mut self.prefs.cf_tunnel_token)
                                    .password(true)
                                    .hint_text("eyJhIjoi...")
                                    .desired_width(316.0),
                            )
                            .changed()
                        {
                            prefs_dirty = true;
                        }
                        ui.small("연결된 고정 도메인 (표시/복사용, 선택)");
                        if ui
                            .add(
                                egui::TextEdit::singleline(&mut self.prefs.cf_tunnel_domain)
                                    .hint_text("예: figcraft.mydomain.com")
                                    .desired_width(316.0),
                            )
                            .changed()
                        {
                            prefs_dirty = true;
                        }
                    }
                    TunnelProvider::Ngrok => {
                        ui.small("ngrok Authtoken (dashboard.ngrok.com)");
                        if ui
                            .add(
                                egui::TextEdit::singleline(&mut self.prefs.ngrok_authtoken)
                                    .password(true)
                                    .hint_text("2abc...")
                                    .desired_width(316.0),
                            )
                            .changed()
                        {
                            prefs_dirty = true;
                        }
                        ui.small("무료 고정 도메인 (비우면 자동 할당)");
                        if ui
                            .add(
                                egui::TextEdit::singleline(&mut self.prefs.ngrok_domain)
                                    .hint_text("예: my-name.ngrok-free.app")
                                    .desired_width(316.0),
                            )
                            .changed()
                        {
                            prefs_dirty = true;
                        }
                    }
                }
                if prefs_dirty {
                    self.services.save_prefs(&self.prefs);
                }

                ui.add_space(8.0);
                let is_running = self.tunnel_state == "connected" || self.tunnel_state == "connecting";
                ui.horizontal(|ui| {
                    if is_running {
                        if ui
                            .add(egui::Button::new("HTTPS 터널 끄기").min_size(egui::vec2(150.0, 34.0)))
                            .clicked()
                        {
                            self.stop_active_tunnel();
                        }
                    } else if ui
                        .add(egui::Button::new("HTTPS 터널 켜기").min_size(egui::vec2(150.0, 34.0)))
                        .clicked()
                    {
                        self.start_selected_tunnel();
                    }

                    let (st_col, st_txt) = match self.tunnel_state.as_str() {
                        "connected" => (egui::Color32::from_rgb(72, 224, 134), "연결됨"),
                        "connecting" => (egui::Color32::from_rgb(245, 194, 84), "연결 중..."),
                        "error" => (egui::Color32::from_rgb(245, 110, 110), "오류"),
                        _ => (egui::Color32::from_rgb(150, 158, 170), "꺼짐"),
                    };
                    ui.colored_label(st_col, st_txt);
                });

                if !self.tunnel_msg.is_empty() {
                    ui.small(&self.tunnel_msg);
                }

                if !self.tunnel_url.is_empty() {
                    ui.add_space(6.0);
                    egui::Frame::group(ui.style())
                        .inner_margin(egui::Margin::same(8))
                        .show(ui, |ui| {
                            ui.small("공개 HTTPS MCP 주소");
                            ui.strong(&self.tunnel_url);
                            ui.horizontal(|ui| {
                                if ui.button("HTTPS 주소 복사").clicked() {
                                    self.services.set_clipboard_text(&self.tunnel_url);
                                    ctx.copy_text(self.tunnel_url.clone());
                                    self.status_banner = Some("HTTPS MCP 주소 복사됨".into());
                                }
                                if ui.button("Gemini CLI JSON 복사").clicked() {
                                    let cfg = json!({
                                        "mcpServers": {
                                            "figcraft": {
                                                "httpUrl": self.tunnel_url,
                                                "headers": {
                                                    "Authorization": format!("Bearer {bearer_token}")
                                                }
                                            }
                                        }
                                    });
                                    let txt = serde_json::to_string_pretty(&cfg).unwrap_or_default();
                                    self.services.set_clipboard_text(&txt);
                                    ctx.copy_text(txt);
                                    self.status_banner = Some("Gemini CLI 설정 JSON 복사됨".into());
                                }
                            });
                        });
                }
            });
        self.mcp_window_open = open;
    }

    fn show_toolbar_editor_window(&mut self, ctx: &egui::Context) {
        if !self.show_toolbar_editor {
            return;
        }
        let mut open = self.show_toolbar_editor;
        let mut changed = false;
        let pos = Self::clamped_popup_pos(ctx, self.toolbar_editor_anchor, 380.0, 440.0);

        egui::Window::new("상단바 버튼 편집")
            .id(egui::Id::unique(("toolbar_editor_win", self.toolbar_editor_open_seq)))
            .default_pos(pos)
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(380.0)
            .show(ctx, |ui| {
                ui.label(egui::RichText::new("좌측 고정 영역 (스크롤해도 항상 고정)").strong());
                ui.add_space(4.0);

                let mut pin_left: Option<usize> = None;
                let mut pin_right: Option<usize> = None;
                let mut unpin_idx: Option<usize> = None;
                let mut pin_remove: Option<usize> = None;

                if self.prefs.pinned_toolbar_buttons.is_empty() {
                    ui.label(egui::RichText::new("고정된 버튼이 없습니다. 아래에서 '고정'을 눌러 추가하세요.").size(11.5).weak());
                } else {
                    let plen = self.prefs.pinned_toolbar_buttons.len();
                    for (idx, btn) in self.prefs.pinned_toolbar_buttons.iter().copied().enumerate() {
                        ui.horizontal(|ui| {
                            if ui.add_enabled(idx > 0, egui::Button::new("◀").min_size(egui::vec2(28.0, 26.0))).clicked() {
                                pin_left = Some(idx);
                            }
                            if ui.add_enabled(idx + 1 < plen, egui::Button::new("▶").min_size(egui::vec2(28.0, 26.0))).clicked() {
                                pin_right = Some(idx);
                            }
                            ui.label(btn.display_name());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.add(egui::Button::new("✕").min_size(egui::vec2(26.0, 26.0))).clicked() {
                                    pin_remove = Some(idx);
                                }
                                if ui.add(egui::Button::new("해제").min_size(egui::vec2(44.0, 26.0))).clicked() {
                                    unpin_idx = Some(idx);
                                }
                            });
                        });
                    }
                }

                if let Some(i) = pin_left {
                    self.prefs.pinned_toolbar_buttons.swap(i - 1, i);
                    changed = true;
                } else if let Some(i) = pin_right {
                    self.prefs.pinned_toolbar_buttons.swap(i, i + 1);
                    changed = true;
                } else if let Some(i) = unpin_idx {
                    let b = self.prefs.pinned_toolbar_buttons.remove(i);
                    self.prefs.toolbar_buttons.insert(0, b);
                    changed = true;
                } else if let Some(i) = pin_remove {
                    self.prefs.pinned_toolbar_buttons.remove(i);
                    changed = true;
                }

                ui.separator();
                ui.label(egui::RichText::new("스크롤 영역 (◀/▶ 순서 이동 · 고정 · ✕ 숨기기)").strong());
                ui.add_space(4.0);

                let mut move_left: Option<usize> = None;
                let mut move_right: Option<usize> = None;
                let mut pin_from_scroll: Option<usize> = None;
                let mut remove_idx: Option<usize> = None;

                egui::ScrollArea::vertical().max_height(210.0).show(ui, |ui| {
                    let len = self.prefs.toolbar_buttons.len();
                    for (idx, btn) in self.prefs.toolbar_buttons.iter().copied().enumerate() {
                        ui.horizontal(|ui| {
                            if ui.add_enabled(idx > 0, egui::Button::new("◀").min_size(egui::vec2(28.0, 26.0))).clicked() {
                                move_left = Some(idx);
                            }
                            if ui.add_enabled(idx + 1 < len, egui::Button::new("▶").min_size(egui::vec2(28.0, 26.0))).clicked() {
                                move_right = Some(idx);
                            }
                            ui.label(btn.display_name());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.add(egui::Button::new("✕").min_size(egui::vec2(26.0, 26.0))).clicked() {
                                    remove_idx = Some(idx);
                                }
                                if ui.add(egui::Button::new("고정").min_size(egui::vec2(44.0, 26.0))).clicked() {
                                    pin_from_scroll = Some(idx);
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
                } else if let Some(i) = pin_from_scroll {
                    let b = self.prefs.toolbar_buttons.remove(i);
                    self.prefs.pinned_toolbar_buttons.push(b);
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
                        if !self.prefs.toolbar_buttons.contains(&btn)
                            && !self.prefs.pinned_toolbar_buttons.contains(&btn)
                        {
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
                        self.prefs.pinned_toolbar_buttons.clear();
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
        if self.tablet_input.mouse_cursor_mode {
            self.tablet_input.virtual_cursor_pos = self.tablet_input.screen_rect.center();
        }
        self.status_banner = Some(if self.tablet_input.mouse_cursor_mode {
            "마우스 모드 켜짐 (우하단 그립으로 조작)".into()
        } else {
            "마우스 모드 꺼짐 (직접 터치)".into()
        });
    }

    fn is_variable_font_file(path: &std::path::Path) -> bool {
        let Ok(mut f) = std::fs::File::open(path) else { return false; };
        let mut buf = [0u8; 4096];
        let Ok(n) = std::io::Read::read(&mut f, &mut buf) else { return false; };
        buf[..n].windows(4).any(|w| w == b"fvar")
    }

    /// Reads OpenType `name` table to extract Korean (0x0412) and English (0x0409) family names.
    fn read_opentype_family_names(path: &std::path::Path) -> (Option<String>, Option<String>) {
        let Ok(bytes) = std::fs::read(path) else { return (None, None); };
        if bytes.len() < 12 {
            return (None, None);
        }
        let sfnt_offset = if &bytes[0..4] == b"ttcf" && bytes.len() >= 16 {
            u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize
        } else {
            0
        };
        if sfnt_offset + 12 > bytes.len() {
            return (None, None);
        }
        let num_tables = u16::from_be_bytes([bytes[sfnt_offset + 4], bytes[sfnt_offset + 5]]) as usize;
        let mut name_off = 0usize;
        let mut name_len = 0usize;
        for i in 0..num_tables {
            let rec = sfnt_offset + 12 + i * 16;
            if rec + 16 > bytes.len() {
                break;
            }
            if &bytes[rec..rec + 4] == b"name" {
                name_off = u32::from_be_bytes([bytes[rec + 8], bytes[rec + 9], bytes[rec + 10], bytes[rec + 11]]) as usize;
                name_len = u32::from_be_bytes([bytes[rec + 12], bytes[rec + 13], bytes[rec + 14], bytes[rec + 15]]) as usize;
                break;
            }
        }
        if name_off == 0 || name_off + 6 > bytes.len() || name_off + name_len > bytes.len() {
            return (None, None);
        }
        let count = u16::from_be_bytes([bytes[name_off + 2], bytes[name_off + 3]]) as usize;
        let str_off = name_off + u16::from_be_bytes([bytes[name_off + 4], bytes[name_off + 5]]) as usize;
        let mut ko_name: Option<String> = None;
        let mut en_name: Option<String> = None;

        for i in 0..count {
            let r = name_off + 6 + i * 12;
            if r + 12 > bytes.len() {
                break;
            }
            let platform_id = u16::from_be_bytes([bytes[r], bytes[r + 1]]);
            let encoding_id = u16::from_be_bytes([bytes[r + 2], bytes[r + 3]]);
            let lang_id = u16::from_be_bytes([bytes[r + 4], bytes[r + 5]]);
            let name_id = u16::from_be_bytes([bytes[r + 6], bytes[r + 7]]);
            let len = u16::from_be_bytes([bytes[r + 8], bytes[r + 9]]) as usize;
            let off = u16::from_be_bytes([bytes[r + 10], bytes[r + 11]]) as usize;
            if name_id != 1 && name_id != 16 {
                continue;
            }
            let start = str_off.saturating_add(off);
            let end = start.saturating_add(len);
            if end > bytes.len() || len == 0 {
                continue;
            }
            if platform_id == 3 && (encoding_id == 1 || encoding_id == 10) && len % 2 == 0 {
                let u16s: Vec<u16> = bytes[start..end]
                    .chunks_exact(2)
                    .map(|c| u16::from_be_bytes([c[0], c[1]]))
                    .collect();
                if let Ok(decoded) = String::from_utf16(&u16s) {
                    let trimmed = decoded.trim().to_owned();
                    if !trimmed.is_empty() {
                        if lang_id == 0x0412 && (ko_name.is_none() || name_id == 16) {
                            ko_name = Some(trimmed);
                        } else if lang_id == 0x0409 && (en_name.is_none() || name_id == 16) {
                            en_name = Some(trimmed);
                        }
                    }
                }
            }
        }
        (ko_name, en_name)
    }

    fn korean_alias(family: &str) -> Option<&'static str> {
        let l = family.to_lowercase();
        if l.contains("pretendard") { Some("프리텐다드") }
        else if l.contains("notosanscjk") || l.contains("notosanskr") { Some("노토 산스 한글") }
        else if l.contains("notoserifcjk") || l.contains("notoserifkr") { Some("노토 세리프 한글") }
        else if l.contains("nanumgothic") { Some("나눔고딕") }
        else if l.contains("nanummyeongjo") { Some("나눔명조") }
        else if l.contains("nanumsquare") { Some("나눔스퀘어") }
        else if l.contains("maruburi") { Some("마루부리") }
        else if l.contains("spoqa") { Some("스포카 한 산스") }
        else if l.contains("gmarket") { Some("지마켓 산스") }
        else if l.contains("roboto") { Some("로보토") }
        else if l.contains("inter") { Some("인터") }
        else { None }
    }

    fn classify_font_name(family: &str, display_name: &str) -> (bool, FontCategory) {
        let lower = family.to_lowercase();
        let has_hangul = display_name.chars().any(|c| ('\u{AC00}'..='\u{D7A3}').contains(&c))
            || family.chars().any(|c| ('\u{AC00}'..='\u{D7A3}').contains(&c));
        let is_korean = has_hangul
            || lower.contains("kr")
            || lower.contains("cjk")
            || lower.contains("hangul")
            || lower.contains("korean")
            || lower.contains("pretendard")
            || lower.contains("nanum")
            || lower.contains("spoqa")
            || lower.contains("gmarket")
            || lower.contains("suit")
            || lower.contains("maruburi")
            || lower.contains("batang")
            || lower.contains("gulim")
            || lower.contains("dotum")
            || lower.contains("malgun")
            || lower.contains("baemin");
        let style = if lower.contains("mono") || lower.contains("code") || lower.contains("console") || lower.contains("courier") {
            FontCategory::Mono
        } else if lower.contains("serif") || lower.contains("myeongjo") || lower.contains("batang") || lower.contains("maruburi") || lower.contains("georgia") || lower.contains("times") {
            FontCategory::Serif
        } else {
            FontCategory::Sans
        };
        (is_korean, style)
    }

    fn refresh_font_catalog(&mut self) {
        let mut entries: Vec<FontCatalogEntry> = Vec::new();
        let mut seen = std::collections::HashSet::new();

        let mut scan_dir = |dir: &std::path::Path, tag: &'static str| {
            let Ok(rd) = std::fs::read_dir(dir) else { return; };
            for entry in rd.flatten() {
                let path = entry.path();
                let Some(ext) = path.extension().and_then(|e| e.to_str()) else { continue; };
                let ext_l = ext.to_lowercase();
                if !matches!(ext_l.as_str(), "ttf" | "otf" | "ttc" | "otc") {
                    continue;
                }
                let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").trim();
                if stem.is_empty() {
                    continue;
                }
                let (ko_name, en_name) = Self::read_opentype_family_names(&path);
                let fallback_fam = stem
                    .split('-')
                    .next()
                    .unwrap_or(stem)
                    .replace('_', " ")
                    .trim()
                    .to_owned();
                let family = en_name.unwrap_or(fallback_fam);
                if family.is_empty() || !seen.insert(family.to_lowercase()) {
                    continue;
                }
                let display_name = if let Some(ko) = ko_name {
                    if ko.eq_ignore_ascii_case(&family) {
                        ko
                    } else {
                        format!("{ko} ({family})")
                    }
                } else if let Some(alias) = Self::korean_alias(&family) {
                    format!("{alias} ({family})")
                } else {
                    family.clone()
                };
                let is_variable = Self::is_variable_font_file(&path)
                    || stem.to_lowercase().contains("variable")
                    || stem.to_lowercase().contains("vf");
                let (is_korean, style) = Self::classify_font_name(&family, &display_name);
                entries.push(FontCatalogEntry {
                    family,
                    display_name,
                    is_variable,
                    is_korean,
                    style,
                    source_tag: tag,
                });
            }
        };

        if let Some(dir) = &self.java_files_dir {
            scan_dir(&dir.join("fonts"), "사용자");
        }
        scan_dir(&self.services.staged_fonts_dir, "사용자");
        scan_dir(std::path::Path::new("/system/fonts"), "시스템");
        scan_dir(std::path::Path::new("/product/fonts"), "시스템");

        entries.sort_by(|a, b| {
            let a_user = a.source_tag == "사용자";
            let b_user = b.source_tag == "사용자";
            b_user
                .cmp(&a_user)
                .then_with(|| b.is_korean.cmp(&a.is_korean))
                .then_with(|| b.is_variable.cmp(&a.is_variable))
                .then_with(|| a.display_name.to_lowercase().cmp(&b.display_name.to_lowercase()))
        });
        self.font_catalog = entries;
    }

    fn show_font_browser_window(&mut self, ctx: &egui::Context) {
        if !self.font_browser_open {
            return;
        }
        let mut open = self.font_browser_open;
        let mut pick_folder = false;
        let mut apply_family: Option<String> = None;
        let pos = Self::clamped_popup_pos(ctx, self.font_browser_anchor, 420.0, 460.0);

        egui::Window::new("폰트 분류 탐색기")
            .id(egui::Id::unique(("font_browser_win", self.font_browser_open_seq)))
            .default_pos(pos)
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(420.0)
            .default_height(460.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("폴더에서 폰트 추가").clicked() {
                        pick_folder = true;
                    }
                    ui.add(
                        egui::TextEdit::singleline(&mut self.font_search)
                            .desired_width(210.0)
                            .hint_text("폰트 검색 (탭해서 입력)"),
                    );
                });
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    for &cat in FontCategory::ALL {
                        if ui.selectable_label(self.font_filter == cat, cat.label()).clicked() {
                            self.font_filter = cat;
                        }
                    }
                });
                ui.separator();

                let q = self.font_search.trim().to_lowercase();
                let list = self.font_catalog.clone();
                egui::ScrollArea::vertical().max_height(330.0).show(ui, |ui| {
                    let mut shown = 0usize;
                    for item in &list {
                        let cat_ok = match self.font_filter {
                            FontCategory::All => true,
                            FontCategory::Variable => item.is_variable,
                            FontCategory::Korean => item.is_korean,
                            other => item.style == other,
                        };
                        if !cat_ok {
                            continue;
                        }
                        if !q.is_empty()
                            && !item.family.to_lowercase().contains(&q)
                            && !item.display_name.to_lowercase().contains(&q)
                        {
                            continue;
                        }
                        shown += 1;
                        ui.horizontal(|ui| {
                            if ui.button(&item.display_name).clicked() {
                                apply_family = Some(item.family.clone());
                            }
                            if item.is_variable {
                                ui.label(egui::RichText::new("VF").size(10.5).color(egui::Color32::from_rgb(76, 175, 80)));
                            }
                            if item.is_korean {
                                ui.label(egui::RichText::new("한글").size(10.5).color(egui::Color32::from_rgb(100, 181, 246)));
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(egui::RichText::new(item.source_tag).size(10.5).weak());
                            });
                        });
                    }
                    if shown == 0 {
                        ui.label("조건에 맞는 폰트가 없습니다. 위 '폴더에서 폰트 추가'로 .ttf/.otf 폴더를 불러오세요.");
                    }
                });
            });

        self.font_browser_open = open;
        if pick_folder {
            let _ = self.services.request_pick_fonts_dir();
        }
        if let Some(fam) = apply_family {
            let _ = self.fig.session.execute("text.rescanFonts", &json!({}));
            let ok = self
                .fig
                .session
                .execute("text.style", &json!({ "fontFamily": fam }))
                .is_ok()
                || self
                    .fig
                    .session
                    .execute("node.set", &json!({ "fontFamily": fam }))
                    .is_ok();
            if ok {
                let _ = self.fig.session.execute("text.refit", &json!({}));
                self.status_banner = Some(format!("폰트 적용: {fam}"));
            } else {
                self.status_banner = Some("먼저 캔버스에서 텍스트 레이어를 선택해 주세요".into());
            }
        }
    }

    fn show_mouse_settings_window(&mut self, ctx: &egui::Context) {
        if !self.show_mouse_settings {
            return;
        }
        let mut open = self.show_mouse_settings;
        let mut changed = false;

        egui::Window::new("마우스 커서 설정")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(350.0)
            .show(ctx, |ui| {
                ui.label("마우스 커서 크기");
                if ui
                    .add(
                        egui::Slider::new(&mut self.prefs.mouse_cursor_scale, 0.6..=2.0)
                            .suffix("x")
                            .fixed_decimals(2),
                    )
                    .changed()
                {
                    self.tablet_input.mouse_cursor_scale = self.prefs.mouse_cursor_scale;
                    changed = true;
                }
                ui.add_space(6.0);

                ui.label("마우스 ↔ 원 링 X 간격");
                if ui
                    .add(
                        egui::Slider::new(&mut self.prefs.mouse_grip_offset_x, -200.0..=200.0)
                            .suffix(" pt")
                            .fixed_decimals(0),
                    )
                    .changed()
                {
                    self.tablet_input.mouse_grip_offset_x = self.prefs.mouse_grip_offset_x;
                    changed = true;
                }
                ui.add_space(4.0);

                ui.label("마우스 ↔ 원 링 Y 간격");
                if ui
                    .add(
                        egui::Slider::new(&mut self.prefs.mouse_grip_offset_y, -200.0..=200.0)
                            .suffix(" pt")
                            .fixed_decimals(0),
                    )
                    .changed()
                {
                    self.tablet_input.mouse_grip_offset_y = self.prefs.mouse_grip_offset_y;
                    changed = true;
                }
                ui.add_space(6.0);

                ui.label("원 링 투명도 (원 링에만 적용)");
                let mut opacity_pct = (self.prefs.mouse_grip_opacity * 100.0).round();
                if ui
                    .add(
                        egui::Slider::new(&mut opacity_pct, 5.0..=100.0)
                            .suffix("%")
                            .fixed_decimals(0),
                    )
                    .changed()
                {
                    self.prefs.mouse_grip_opacity = (opacity_pct / 100.0).clamp(0.05, 1.0);
                    self.tablet_input.mouse_grip_opacity = self.prefs.mouse_grip_opacity;
                    changed = true;
                }
                ui.add_space(6.0);

                ui.label("테두리 굵기 (마우스 · 원 링)");
                if ui
                    .add(
                        egui::Slider::new(&mut self.prefs.mouse_border_width, 0.5..=5.0)
                            .suffix(" pt")
                            .fixed_decimals(1),
                    )
                    .changed()
                {
                    self.tablet_input.mouse_border_width = self.prefs.mouse_border_width;
                    changed = true;
                }
                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    if ui.button("기본값 복원").clicked() {
                        self.prefs.mouse_cursor_scale = 1.0;
                        self.prefs.mouse_grip_offset_x = 56.0;
                        self.prefs.mouse_grip_offset_y = 56.0;
                        self.prefs.mouse_grip_opacity = 0.45;
                        self.prefs.mouse_border_width = 2.0;
                        self.tablet_input.mouse_cursor_scale = 1.0;
                        self.tablet_input.mouse_grip_offset_x = 56.0;
                        self.tablet_input.mouse_grip_offset_y = 56.0;
                        self.tablet_input.mouse_grip_opacity = 0.45;
                        self.tablet_input.mouse_border_width = 2.0;
                        changed = true;
                    }
                });
            });

        if changed {
            self.services.save_prefs(&self.prefs);
        }
        self.show_mouse_settings = open;
    }

    fn toggle_mcp_lan(&mut self) {
        let next_lan = !self.mcp_lan_mode;
        let bind_str = if next_lan { "0.0.0.0:47800" } else { "127.0.0.1:47800" };
        let Ok(bind_addr) = bind_str.parse::<SocketAddr>() else {
            return;
        };

        self.mcp_server = None;
        if self.prefs.mcp_bearer_token.trim().is_empty() {
            self.prefs.mcp_bearer_token = mcp_http::generate_session_token();
            self.services.save_prefs(&self.prefs);
        }
        let cfg = McpServerConfig::with_token(bind_addr, self.prefs.mcp_bearer_token.clone());
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
                let msg = match res {
                    Ok(bytes) => LoadedAsset::Image {
                        staged_path,
                        display_name,
                        mime,
                        bytes,
                    },
                    Err(err) => {
                        let _ = std::fs::remove_file(&staged_path);
                        LoadedAsset::Err {
                            display_name,
                            error: err.to_string(),
                        }
                    }
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
                ActivityEvent::ImeState(enabled) => {
                    if !enabled {
                        self.ime_composing = false;
                    }
                }
                ActivityEvent::ImePreedit(text) => {
                    self.ime_composing = !text.is_empty();
                    self.pending_egui_events.push(egui::Event::Ime(egui::ImeEvent::Preedit {
                        text,
                        active_range_chars: None,
                    }));
                }
                ActivityEvent::ImeCommit(text) => {
                    let preview: String = text.chars().take(12).collect();
                    self.status_banner = Some(format!("IME: {preview}"));
                    if self.ime_composing {
                        self.ime_composing = false;
                        self.pending_egui_events.push(egui::Event::Ime(egui::ImeEvent::Commit(text)));
                    } else {
                        self.pending_egui_events.push(egui::Event::Text(text));
                    }
                }
                ActivityEvent::ImeKey(key_name) => {
                    match key_name.as_str() {
                        "backspace" => self.queue_key_shortcut(egui::Key::Backspace, false, false),
                        "enter" => self.queue_key_shortcut(egui::Key::Enter, false, false),
                        "tab" => self.queue_key_shortcut(egui::Key::Tab, false, false),
                        "escape" => self.queue_key_shortcut(egui::Key::Escape, false, false),
                        _ => {}
                    }
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
                ActivityEvent::DropImage {
                    path,
                    display_name,
                    mime,
                } => {
                    self.status_banner = Some(format!("이미지 배치 중: {display_name}"));
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
                    let _ = self.fig.session.execute("text.rescanFonts", &json!({}));
                    let _ = self.fig.session.execute("text.refit", &json!({}));
                    self.refresh_font_catalog();
                    self.ui_font_frame_count = 0;
                    self.status_banner = Some(format!("사용자 폰트 {count}개 동기화 및 등록 완료"));
                }
                ActivityEvent::UiFontPicked { path, display_name } => {
                    self.pending_ui_font_path = Some(PathBuf::from(path));
                    self.status_banner = Some(format!("UI Font: {display_name}"));
                }
                ActivityEvent::FilesDir(dir) => {
                    self.java_files_dir = Some(PathBuf::from(dir));
                    self.ui_font_frame_count = 0;
                    let _ = self.fig.session.execute("text.rescanFonts", &json!({}));
                }
                ActivityEvent::SystemTheme { dark } => {
                    self.android_system_dark = Some(dark);
                }
                ActivityEvent::AppPaused => {
                    self.save_recovery_snapshots(false);
                }
                ActivityEvent::McpServiceStopped => {
                    self.mcp_server = None;
                    self.mcp_lan_mode = false;
                    self.status_banner = Some("MCP stopped from notification".into());
                }
                ActivityEvent::DriveAuth { signed_in, email } => {
                    self.drive_signed_in = signed_in;
                    self.drive_email = email;
                }
                ActivityEvent::DriveIndex(raw) => {
                    if let Ok(parsed) = serde_json::from_str::<Value>(&raw) {
                        if let Some(arr) = parsed.get("items").and_then(|v| serde_json::from_value::<Vec<DriveCanvasItem>>(v.clone()).ok()) {
                            self.drive_items = arr;
                        }
                    }
                }
                ActivityEvent::DriveStatus(msg) => {
                    if !msg.is_empty() {
                        self.drive_status = msg.clone();
                        self.status_banner = Some(msg);
                    }
                }
                ActivityEvent::DriveCreds { client_id, client_secret } => {
                    self.prefs.drive_client_id = client_id;
                    self.prefs.drive_client_secret = client_secret;
                    self.services.save_prefs(&self.prefs);
                    self.drive_status = "JSON 불러오기 완료! 'Google 로그인'을 눌러 연결하세요.".into();
                }
                ActivityEvent::NativeText(text) => {
                    self.pending_paste_text = Some(text);
                    self.status_banner = Some("텍스트 입력 완료".into());
                }
                ActivityEvent::RenameCanvas(new_name) => {
                    let mut renamed = false;
                    if let Some(doc_state) = self.fig.session.docs.get_mut(self.fig.session.active) {
                        Arc::make_mut(&mut doc_state.doc).name = new_name.clone();
                        renamed = true;
                    }
                    if renamed {
                        self.status_banner = Some(format!("캔버스 이름 변경: {new_name}"));
                        self.save_recovery_snapshots(true);
                    }
                }
                ActivityEvent::TunnelState { state, url, message } => {
                    self.tunnel_state = state;
                    if !url.is_empty() || self.tunnel_state == "stopped" || self.tunnel_state == "error" {
                        self.tunnel_url = url;
                    }
                    self.tunnel_msg = message;
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
                    staged_path,
                    display_name,
                    mime,
                    bytes,
                } => {
                    let kb = bytes.len() / 1024;
                    let b64 = crate::services::encode_base64(&bytes);
                    let placed_by_data = self
                        .fig
                        .session
                        .execute("image.place", &json!({ "data": b64, "name": display_name }))
                        .is_ok();
                    if !placed_by_data {
                        self.fig.run("image.place", json!({ "path": staged_path, "name": display_name }));
                    }
                    let _ = std::fs::remove_file(&staged_path);
                    self.status_banner = Some(format!("이미지 배치 완료: {display_name} ({mime}, {kb} KB)"));
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
            "initialize" => {
                if let Some(cname) = req.pointer("/params/clientInfo/name").and_then(Value::as_str) {
                    self.agent_bubble.set_detected_client_name(cname);
                }
                json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": "2025-03-26",
                    "serverInfo": { "name": "figcraft-android-mcp", "version": "0.18.1" },
                    "capabilities": { "tools": {} }
                }
            })
            }
            "tools/list" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "tools": [
                        {
                            "name": "figcraft.status",
                            "description": "Return active document name, open doc count, tunnel URL, paused state, and tablet shell status.",
                            "inputSchema": { "type": "object", "properties": {} }
                        },
                        {
                            "name": "list_commands",
                            "description": "List available FigCraft engine commands and common usage examples (optional filter substring).",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "filter": { "type": "string", "description": "Optional substring filter (e.g. 'node', 'text', 'layout', 'fill')" }
                                }
                            }
                        },
                        {
                            "name": "engine.execute",
                            "description": "Execute any FigCraft engine command on the live canvas (e.g. node.create, node.set, node.get, node.delete, text.create, fill.set, stroke.set, effect.add, layout.add, align.left, distribute.horizontal, component.create, doc.inspect, file.newFromTemplate, edit.undo, edit.redo).",
                            "inputSchema": {
                                "type": "object",
                                "required": ["command"],
                                "properties": {
                                    "command": { "type": "string", "description": "FigCraft engine command id" },
                                    "params": { "type": "object", "description": "JSON parameters for the command" }
                                }
                            }
                        },
                        {
                            "name": "create_node",
                            "description": "Create a frame, rectangle, ellipse, text, polygon, star, line, arrow, vector, section, or component on the live canvas.",
                            "inputSchema": {
                                "type": "object",
                                "required": ["type"],
                                "properties": {
                                    "type": { "type": "string", "description": "frame|rectangle|ellipse|text|polygon|star|line|arrow|vector|section|component" },
                                    "name": { "type": "string" },
                                    "x": { "type": "number" },
                                    "y": { "type": "number" },
                                    "width": { "type": "number" },
                                    "height": { "type": "number" },
                                    "parent": { "type": "integer" },
                                    "text": { "type": "string" },
                                    "props": { "type": "object" }
                                }
                            }
                        },
                        {
                            "name": "set_properties",
                            "description": "Update geometry, fills, strokes, cornerRadius, layout, padding, gap, opacity, text, or typography on layer ids.",
                            "inputSchema": {
                                "type": "object",
                                "required": ["props"],
                                "properties": {
                                    "ids": { "type": "array", "items": { "type": "integer" } },
                                    "props": { "type": "object" }
                                }
                            }
                        },
                        {
                            "name": "inspect_document",
                            "description": "Inspect the active document layer tree, selection, and styles.",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "depth": { "type": "integer" },
                                    "allPages": { "type": "boolean" }
                                }
                            }
                        },
                        {
                            "name": "get_node",
                            "description": "Inspect a single layer by id.",
                            "inputSchema": {
                                "type": "object",
                                "required": ["id"],
                                "properties": {
                                    "id": { "type": "integer" },
                                    "depth": { "type": "integer" }
                                }
                            }
                        },
                        {
                            "name": "undo",
                            "description": "Undo the last edit step on the active document.",
                            "inputSchema": { "type": "object", "properties": {} }
                        },
                        {
                            "name": "redo",
                            "description": "Redo the last undone edit step on the active document.",
                            "inputSchema": { "type": "object", "properties": {} }
                        },
                        {
                            "name": "new_from_template",
                            "description": "Create a new starter design from a built-in template (mobile-app, landing-page, dashboard, design-system, wireframe-kit, social-posts, slides, icon-set).",
                            "inputSchema": {
                                "type": "object",
                                "required": ["template"],
                                "properties": {
                                    "template": { "type": "string" },
                                    "name": { "type": "string" }
                                }
                            }
                        }
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
                let canvas_rect = self.tablet_input.canvas_rect;

                let run_engine = |shell: &mut Self, cmd: &str, cmd_params: &Value, mutating: bool| -> Value {
                    if mutating && shell.agent_bubble.paused {
                        shell.agent_bubble.record_activity_with_target(cmd, false, Some(cmd_params), None, canvas_rect);
                        return json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32001, "message": "AI agent is paused by user" }
                        });
                    }
                    match shell.fig.session.execute(cmd, cmd_params) {
                        Ok(out) => {
                            shell.agent_bubble.record_activity_with_target(cmd, true, Some(cmd_params), None, canvas_rect);
                            json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": {
                                    "content": [{ "type": "text", "text": format!("{out:?}") }]
                                }
                            })
                        }
                        Err(err) => {
                            shell.agent_bubble.record_activity_with_target(cmd, false, Some(cmd_params), None, canvas_rect);
                            json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "error": { "code": -32000, "message": format!("{err}") }
                            })
                        }
                    }
                };

                match name {
                    "engine.execute" => {
                        let cmd = args.get("command").and_then(Value::as_str).unwrap_or("");
                        let cmd_params = args.get("params").cloned().unwrap_or_else(|| json!({}));
                        let is_read = cmd.starts_with("doc.inspect")
                            || cmd.starts_with("node.get")
                            || cmd.starts_with("node.list")
                            || cmd.starts_with("inspect.")
                            || cmd.starts_with("template.list")
                            || cmd.starts_with("page.list")
                            || cmd.starts_with("style.list");
                        run_engine(self, cmd, &cmd_params, !is_read)
                    }
                    "create_node" => run_engine(self, "node.create", &args, true),
                    "set_properties" => {
                        let mut p = args.get("props").cloned().unwrap_or_else(|| json!({}));
                        if let Some(ids) = args.get("ids") {
                            if let Some(obj) = p.as_object_mut() {
                                obj.insert("ids".into(), ids.clone());
                            }
                        }
                        run_engine(self, "node.set", &p, true)
                    }
                    "inspect_document" => run_engine(self, "doc.inspect", &args, false),
                    "get_node" => run_engine(self, "node.get", &args, false),
                    "undo" => run_engine(self, "edit.undo", &json!({}), true),
                    "redo" => run_engine(self, "edit.redo", &json!({}), true),
                    "new_from_template" => run_engine(self, "file.newFromTemplate", &args, true),
                    "list_commands" => {
                        self.agent_bubble.record_activity_with_target("doc.inspect", true, None, None, canvas_rect);
                        let filter = args.get("filter").and_then(Value::as_str).unwrap_or("").to_ascii_lowercase();
                        let catalog = [
                            ("node.create", "Create frame/rectangle/ellipse/text/vector/component {type, name?, x?, y?, width?, height?, parent?, text?, props?}"),
                            ("node.set", "Update layer properties {ids?, x?, y?, width?, height?, fill?, stroke?, cornerRadius?, opacity?, layout?, padding?, gap?, text?, fontSize?, fontFamily?, fontStyle?}"),
                            ("node.get", "Inspect one layer {id, depth?}"),
                            ("node.delete", "Delete layers {ids?}"),
                            ("node.move", "Move layers {ids?, x, y}"),
                            ("node.resize", "Resize layers {ids?, width, height}"),
                            ("doc.inspect", "Inspect full document tree, selection, and styles {depth?, allPages?}"),
                            ("text.create", "Create text layer {text, x?, y?, fontSize?, fontFamily?, fill?}"),
                            ("text.set", "Edit text contents {id?, text}"),
                            ("text.style", "Style text range {id?, start?, end?, fontSize?, fontFamily?, fontStyle?, fill?}"),
                            ("fill.set", "Set layer fill paint {ids?, color|paint}"),
                            ("stroke.set", "Set layer stroke {ids?, color?, width?}"),
                            ("effect.add", "Add dropShadow/innerShadow/layerBlur/backgroundBlur/glass {ids?, type, radius?, offset?, color?}"),
                            ("layout.add", "Add auto layout to frame {ids?, direction?, gap?, padding?}"),
                            ("align.left / align.centerH / align.top / align.centerV", "Align selected layers {ids?}"),
                            ("distribute.horizontal / distribute.vertical", "Distribute selected layers evenly {ids?}"),
                            ("component.create", "Turn layers into a component {ids?}"),
                            ("instance.create", "Place component instance {component, x?, y?}"),
                            ("image.place", "Place image on canvas {path|data, name?, x?, y?}"),
                            ("file.newFromTemplate", "Create starter template {template: mobile-app|landing-page|dashboard|design-system|wireframe-kit|social-posts|slides|icon-set, name?}"),
                            ("template.list", "List available starter templates {}"),
                            ("view.zoomToFit", "Zoom viewport to fit page {}"),
                            ("edit.undo / edit.redo", "Undo or redo last edit step {}"),
                            ("edit.duplicate / edit.delete", "Duplicate or delete selection {}"),
                            ("select.set", "Select layer ids {ids: [...]}"),
                            ("object.group / object.ungroup", "Group or ungroup layers {ids?}"),
                        ];
                        let lines: Vec<String> = catalog
                            .iter()
                            .filter(|(k, d)| {
                                filter.is_empty()
                                    || k.to_ascii_lowercase().contains(&filter)
                                    || d.to_ascii_lowercase().contains(&filter)
                            })
                            .map(|(k, d)| format!("- {k}: {d}"))
                            .collect();
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "content": [{
                                    "type": "text",
                                    "text": format!("FigCraft Mobile MCP Commands ({} matched):
{}", lines.len(), lines.join("
"))
                                }]
                            }
                        })
                    }
                    _ => {
                        self.agent_bubble.record_activity_with_target("figcraft.status", true, None, None, canvas_rect);
                        let active_doc_name = self
                            .fig
                            .session
                            .docs
                            .get(self.fig.session.active)
                            .map(|d| d.doc.name.as_str())
                            .unwrap_or("(none)");
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "content": [{
                                    "type": "text",
                                    "text": format!(
                                        "FigCraft Mobile v29 active (active_doc=\"{}\", docs={}, local={}, https_tunnel=\"{}\", paused={}, mouse={})",
                                        active_doc_name,
                                        self.fig.session.docs.len(),
                                        self.mcp_endpoint_label,
                                        if self.tunnel_url.is_empty() { "off" } else { self.tunnel_url.as_str() },
                                        self.agent_bubble.paused,
                                        self.tablet_input.mouse_cursor_mode
                                    )
                                }]
                            }
                        })
                    }
                }
            }
            _ => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": format!("Method not found: {method}") }
            }),
        }
    }

    fn upload_active_canvas_to_drive(&mut self) {
        let Some(dir) = self.java_files_dir.clone() else { return; };
        let cache_dir = dir.join("drive_cache");
        let _ = std::fs::create_dir_all(&cache_dir);
        let Some(doc_state) = self.fig.session.docs.get(self.fig.session.active) else { return; };
        let raw_name = if doc_state.doc.name.trim().is_empty() { "Untitled" } else { doc_state.doc.name.trim() };
        let safe_name: String = raw_name
            .chars()
            .map(|c| if matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
            .collect();
        let file_name = if safe_name.ends_with(".figcraft") { safe_name } else { format!("{safe_name}.figcraft") };
        let local_fig = cache_dir.join(&file_name);
        let (doc, versions, branches) = recovery_copy(&self.fig.session, doc_state);
        if let Ok(bytes) = figcraft_format::save_bytes_with_history(&doc, None, &versions, &branches) {
            if std::fs::write(&local_fig, bytes).is_ok() {
                self.services.drive_upload_canvas(doc_state.uid, &file_name, &local_fig.to_string_lossy(), "");
            }
        }
    }

    fn paint_canvas_card_preview(ui: &mut egui::Ui, size: egui::Vec2, title: &str, synced: bool) {
        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
        let p = ui.painter();
        p.rect_filled(rect, 8.0, ui.visuals().extreme_bg_color);
        p.rect_stroke(rect, 8.0, egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color), egui::StrokeKind::Inside);
        let bar = egui::Rect::from_min_size(rect.min + egui::vec2(10.0, 10.0), egui::vec2(size.x - 20.0, 10.0));
        p.rect_filled(bar, 3.0, egui::Color32::from_rgba_unmultiplied(76, 175, 80, 70));
        let box1 = egui::Rect::from_min_size(rect.min + egui::vec2(10.0, 26.0), egui::vec2((size.x - 26.0) * 0.55, size.y - 36.0));
        let box2 = egui::Rect::from_min_size(egui::pos2(box1.max.x + 6.0, box1.min.y), egui::vec2((size.x - 26.0) * 0.45, size.y - 36.0));
        p.rect_filled(box1, 4.0, ui.visuals().faint_bg_color);
        p.rect_filled(box2, 4.0, ui.visuals().faint_bg_color);
        let badge = if synced { "Drive" } else { "로컬" };
        p.text(
            rect.right_bottom() - egui::vec2(8.0, 6.0),
            egui::Align2::RIGHT_BOTTOM,
            badge,
            egui::FontId::proportional(10.0),
            egui::Color32::from_rgb(76, 175, 80),
        );
        let _ = title;
    }

    fn show_home_drive_strip(&mut self, ui: &mut egui::Ui) {
        let mut open_req: Option<(String, String)> = None;
        let mut open_page = false;
        let mut do_sync = false;
        egui::Panel::bottom("android_home_drive_recents")
            .resizable(false)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let title_lbl = if self.drive_signed_in {
                        if self.drive_email.is_empty() {
                            "Google Drive (FigCraft 폴더 · 연결됨)".to_string()
                        } else {
                            format!("Google Drive ({})", self.drive_email)
                        }
                    } else {
                        "Google Drive (FigCraft 폴더)".to_string()
                    };
                    ui.label(egui::RichText::new(title_lbl).strong().size(13.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let btn_label = if self.drive_signed_in {
                            "Google Drive 페이지 열기"
                        } else {
                            "Google Drive 연결 / 설정"
                        };
                        let hbtn = ui.button(btn_label);
                        if hbtn.clicked() {
                            open_page = true;
                            self.drive_window_anchor = Some(egui::pos2(hbtn.rect.left() - 360.0, hbtn.rect.top() - 510.0));
                            self.drive_window_open_seq = self.drive_window_open_seq.wrapping_add(1);
                        }
                        if self.drive_signed_in && ui.button("새로고침").clicked() {
                            do_sync = true;
                        }
                    });
                });

                let items = self.drive_items.clone();
                if items.is_empty() {
                    ui.add_space(2.0);
                    let hint = if self.drive_signed_in {
                        "아직 동기화된 캔버스가 없습니다. 캔버스를 편집하거나 'Google Drive 페이지 열기'에서 업로드하세요."
                    } else {
                        "오른쪽 'Google Drive 연결 / 설정'에서 다운로드한 JSON 파일을 불러와 로그인하면 캔버스가 자동 동기화됩니다."
                    };
                    ui.label(egui::RichText::new(hint).size(11.5).weak());
                } else {
                    ui.add_space(4.0);
                    egui::ScrollArea::horizontal().show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for item in items.iter().take(12) {
                                let title = item.name.strip_suffix(".figcraft").unwrap_or(&item.name);
                                egui::Frame::group(ui.style())
                                    .inner_margin(egui::Margin::same(8))
                                    .show(ui, |ui| {
                                        ui.set_width(148.0);
                                        Self::paint_canvas_card_preview(ui, egui::vec2(132.0, 62.0), title, item.synced);
                                        ui.add_space(4.0);
                                        ui.label(egui::RichText::new(title).strong().size(12.5));
                                        if ui.small_button("열기").clicked() {
                                            open_req = Some((item.file_id.clone(), item.name.clone()));
                                        }
                                    });
                            }
                        });
                    });
                }
            });
        if open_page {
            self.drive_window_open = true;
        }
        if do_sync {
            self.services.drive_sync_now();
        }
        if let Some((fid, name)) = open_req {
            self.services.drive_open_file(&fid, &name);
            self.fig.ui.home = false;
        }
    }

    fn show_drive_window(&mut self, ctx: &egui::Context) {
        if !self.drive_window_open {
            return;
        }
        let mut open = self.drive_window_open;
        let mut do_pick_json = false;
        let mut do_open_console = false;
        let mut do_sign_in = false;
        let mut do_sign_out = false;
        let mut do_sync = false;
        let mut do_upload = false;
        let mut do_rename = false;
        let mut prefs_changed = false;
        let mut open_req: Option<(String, String)> = None;
        let mut delete_req: Option<(String, String)> = None;

        let drive_pos = Self::clamped_popup_pos(ctx, self.drive_window_anchor, 540.0, 500.0);
        egui::Window::new("Google Drive (FigCraft 폴더)")
            .id(egui::Id::unique(("drive_sync_win", self.drive_window_open_seq)))
            .default_pos(drive_pos)
            .open(&mut open)
            .resizable(true)
            .default_width(540.0)
            .default_height(500.0)
            .show(ctx, |ui| {
                if self.drive_signed_in {
                    ui.horizontal(|ui| {
                        let who = if self.drive_email.is_empty() { "연결됨".to_string() } else { self.drive_email.clone() };
                        ui.label(egui::RichText::new(format!("계정: {who}")).strong());
                        if ui.button("로그아웃").clicked() {
                            do_sign_out = true;
                        }
                    });
                } else {
                    ui.horizontal(|ui| {
                        if ui.button("JSON 파일 불러오기 (추천)").clicked() {
                            do_pick_json = true;
                        }
                        if ui.button("Google 로그인").clicked() {
                            do_sign_in = true;
                        }
                        if ui.button("Google Cloud 열기").clicked() {
                            do_open_console = true;
                        }
                    });
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label("Client ID:");
                        if ui.add(egui::TextEdit::singleline(&mut self.prefs.drive_client_id).desired_width(360.0).hint_text("JSON 불러오기로 자동 입력")).changed() {
                            prefs_changed = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Client Secret:");
                        if ui.add(egui::TextEdit::singleline(&mut self.prefs.drive_client_secret).password(true).desired_width(340.0).hint_text("JSON 불러오기로 자동 입력")).changed() {
                            prefs_changed = true;
                        }
                    });
                    ui.add_space(4.0);
                    egui::CollapsingHeader::new("처음 설정하는 방법 (3분 튜토리얼)")
                        .default_open(self.prefs.drive_client_id.is_empty())
                        .show(ui, |ui| {
                            ui.label("1. 위 'Google Cloud 열기'에서 프로젝트를 만들고 Google Drive API를 '사용'으로 켭니다.");
                            ui.label("2. Google 인증 플랫폼 → 대상(외부) → 테스트 사용자에 본인 구글 이메일을 추가합니다.");
                            ui.label("3. 클라이언트 → 만들기 → '데스크톱 앱' 선택 (AI 에이전트 체크 안 함) → JSON 다운로드.");
                            ui.label("4. 위 'JSON 파일 불러오기'로 받은 파일을 선택하면 ID/비밀번호가 자동 입력됩니다.");
                            ui.label("5. 'Google 로그인'을 눌러 브라우저에서 승인하면 FigCraft 폴더가 자동 생성·동기화됩니다.");
                            ui.label("   · 403 access_denied 오류 시: 2번 테스트 사용자에 로그인 이메일이 등록되었는지 확인하세요.");
                            ui.label("   · '확인하지 않은 앱' 화면이 뜨면: 왼쪽 아래 '고급' → 'FigCraft Mobile(으)로 이동'을 누르세요.");
                        });
                }

                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut self.prefs.drive_auto_sync, "자동 업로드 및 동기화 (FigCraft 폴더)").changed() {
                        prefs_changed = true;
                    }
                    if self.drive_signed_in {
                        if ui.button("지금 동기화").clicked() {
                            do_sync = true;
                        }
                        if ui.button("현재 캔버스 업로드").clicked() {
                            do_upload = true;
                        }
                        if ui.button("캔버스 이름 변경").clicked() {
                            do_rename = true;
                        }
                    }
                });
                if !self.drive_status.is_empty() {
                    ui.label(egui::RichText::new(&self.drive_status).size(11.5).weak());
                }
                ui.separator();

                let items = self.drive_items.clone();
                if items.is_empty() {
                    ui.label("드라이브의 FigCraft 폴더에 저장된 캔버스가 아직 없습니다.");
                } else {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            for item in &items {
                                let title = item.name.strip_suffix(".figcraft").unwrap_or(&item.name);
                                egui::Frame::group(ui.style())
                                    .inner_margin(egui::Margin::same(8))
                                    .show(ui, |ui| {
                                        ui.set_width(152.0);
                                        Self::paint_canvas_card_preview(ui, egui::vec2(136.0, 72.0), title, item.synced);
                                        ui.add_space(4.0);
                                        ui.label(egui::RichText::new(title).strong().size(13.0));
                                        if !item.modified_time.is_empty() {
                                            let short_date: String = item.modified_time.chars().take(10).collect();
                                            ui.label(egui::RichText::new(short_date).size(11.0).weak());
                                        }
                                        ui.horizontal(|ui| {
                                            if ui.button("열기").clicked() {
                                                open_req = Some((item.file_id.clone(), item.name.clone()));
                                            }
                                            if ui.button("삭제").clicked() {
                                                delete_req = Some((item.file_id.clone(), item.name.clone()));
                                            }
                                        });
                                    });
                            }
                        });
                    });
                }
            });

        self.drive_window_open = open;
        if prefs_changed {
            self.services.save_prefs(&self.prefs);
        }
        if do_pick_json {
            self.services.pick_drive_json();
        }
        if do_open_console {
            self.services.open_url("https://console.cloud.google.com/auth/clients");
        }
        if do_sign_in {
            self.services.save_prefs(&self.prefs);
            self.services.drive_sign_in(&self.prefs.drive_client_id, &self.prefs.drive_client_secret);
        }
        if do_sign_out {
            self.services.drive_sign_out();
        }
        if do_sync {
            self.services.drive_sync_now();
        }
        if do_upload {
            self.upload_active_canvas_to_drive();
        }
        if do_rename {
            let cur = self
                .fig
                .session
                .docs
                .get(self.fig.session.active)
                .map(|d| d.doc.name.clone())
                .unwrap_or_else(|| "Untitled".into());
            self.services.prompt_rename_canvas(&cur);
        }
        if let Some((fid, name)) = open_req {
            self.services.drive_open_file(&fid, &name);
            self.drive_window_open = false;
            self.fig.ui.home = false;
        }
        if let Some((fid, name)) = delete_req {
            self.services.drive_delete_file(&fid, &name);
        }
    }

    fn poll_game_text_input(&mut self) {
        if !self.game_ime_active {
            return;
        }
        let Some((text, compose_region)) = platform::game_poll_text_input() else {
            return;
        };

        let (cur_committed, cur_composing): (&str, &str) =
            if let Some((cs_raw, ce_raw)) = compose_region {
                let cs = cs_raw.min(text.len());
                let ce = ce_raw.min(text.len());
                if cs <= ce && text.is_char_boundary(cs) && text.is_char_boundary(ce) {
                    (&text[..cs], &text[cs..ce])
                } else {
                    (text.as_str(), "")
                }
            } else {
                (text.as_str(), "")
            };

        // 1. Diff committed prefix against previous committed prefix WITHOUT resetting IME buffer
        if cur_committed != self.game_ime_prev_committed {
            let prev_chars: Vec<char> = self.game_ime_prev_committed.chars().collect();
            let cur_chars: Vec<char> = cur_committed.chars().collect();
            let mut common = 0usize;
            while common < prev_chars.len()
                && common < cur_chars.len()
                && prev_chars[common] == cur_chars[common]
            {
                common += 1;
            }
            let deleted = prev_chars.len().saturating_sub(common);
            let added: String = cur_chars[common..].iter().collect();

            if deleted == 0 && !added.is_empty() && !self.game_ime_last_composing.is_empty() {
                // Normal IME commit of composing syllable(s)
                self.game_ime_last_composing.clear();
                self.emit_committed_text_or_enter(&added, true);
            } else {
                if !self.game_ime_last_composing.is_empty() {
                    self.game_ime_last_composing.clear();
                    self.pending_egui_events.push(egui::Event::Ime(egui::ImeEvent::Preedit {
                        text: String::new(),
                        active_range_chars: None,
                    }));
                }
                for _ in 0..deleted.min(32) {
                    self.queue_key_shortcut(egui::Key::Backspace, false, false);
                }
                if !added.is_empty() {
                    self.emit_committed_text_or_enter(&added, false);
                }
            }
            self.game_ime_prev_committed = cur_committed.to_owned();
        }

        // 2. Update live composing (preedit) slice
        if cur_composing != self.game_ime_last_composing {
            self.game_ime_last_composing = cur_composing.to_owned();
            self.pending_egui_events.push(egui::Event::Ime(egui::ImeEvent::Preedit {
                text: self.game_ime_last_composing.clone(),
                active_range_chars: None,
            }));
        }
    }

    fn emit_committed_text_or_enter(&mut self, text: &str, via_ime_commit: bool) {
        let mut first = true;
        for part in text.split('\n') {
            if !first {
                self.queue_key_shortcut(egui::Key::Enter, false, false);
            }
            first = false;
            let clean = part.trim_end_matches('\r');
            if !clean.is_empty() {
                if via_ime_commit {
                    self.pending_egui_events
                        .push(egui::Event::Ime(egui::ImeEvent::Commit(clean.to_owned())));
                } else {
                    self.pending_egui_events
                        .push(egui::Event::Text(clean.to_owned()));
                }
            }
        }
    }
}

impl eframe::App for FigcraftAndroidShell {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        figcraft_ui_egui::windows::guard(&mut self.fig, ctx, |fig| fig.background_tick(ctx));
        if self.last_recovery_save.elapsed() >= std::time::Duration::from_secs(30) {
            self.save_recovery_snapshots(false);
        }
    }

    fn on_exit(&mut self) {
        self.save_recovery_snapshots(true);
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

        self.drain_activity_events();
        self.poll_game_text_input();

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

        let wants_kb = ctx.egui_wants_keyboard_input();
        if !self.restore_focus_before_paste && wants_kb {
            if let Some(focused) = ctx.memory(|m| m.focused()) {
                self.last_focused_id = Some(focused);
            }
        }

        self.drain_activity_events();
        self.drain_mcp_requests();

        // Sync Android status/nav bar icon color with FigCraft light/dark theme
        let is_light_theme = !ui.style().visuals.dark_mode;
        if self.last_light_bars != Some(is_light_theme) {
            self.last_light_bars = Some(is_light_theme);
            self.services.set_light_system_bars(is_light_theme);
            self.ui_font_frame_count = 0;
        }

        let top_inset = self.tablet_input.safe_insets_px[0] / ppp;
        let right_inset = self.tablet_input.safe_insets_px[1] / ppp;
        let bottom_inset = self.tablet_input.safe_insets_px[2] / ppp;
        let left_inset = self.tablet_input.safe_insets_px[3] / ppp;

        let top_btn_h = 44.0;
        let top_resp = egui::Panel::top("figcraft_android_topbar").show(ui, |ui| {
            ui.add_space(top_inset + 3.0);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                ui.add_space(left_inset);

                let pinned = self.prefs.pinned_toolbar_buttons.clone();
                if !pinned.is_empty() {
                    for btn_id in pinned {
                        self.render_toolbar_button(ui, &ctx, btn_id, top_btn_h);
                    }
                    let (sep_rect, _) = ui.allocate_exact_size(egui::vec2(8.0, top_btn_h), egui::Sense::hover());
                    let sx = sep_rect.center().x;
                    ui.painter().line_segment(
                        [egui::pos2(sx, sep_rect.top() + 6.0), egui::pos2(sx, sep_rect.bottom() - 6.0)],
                        egui::Stroke::new(1.5, ui.visuals().widgets.noninteractive.bg_stroke.color),
                    );
                }

                egui::ScrollArea::horizontal()
                    .id_salt("topbar_scroll")
                    .show(ui, |ui| {
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = 7.0;
                            let buttons = self.prefs.toolbar_buttons.clone();
                            for btn_id in buttons {
                                self.render_toolbar_button(ui, &ctx, btn_id, top_btn_h);
                            }
                            let plus_resp = ui.add(
                                egui::Button::new("+")
                                    .selected(self.show_toolbar_editor)
                                    .min_size(egui::vec2(42.0, top_btn_h)),
                            );
                            if plus_resp.clicked() {
                                self.show_toolbar_editor = !self.show_toolbar_editor;
                                if self.show_toolbar_editor {
                                    self.toolbar_editor_anchor = Some(egui::pos2(
                                        plus_resp.rect.left(),
                                        plus_resp.rect.bottom() + 6.0,
                                    ));
                                    self.toolbar_editor_open_seq = self.toolbar_editor_open_seq.wrapping_add(1);
                                }
                            }
                            if let Some(msg) = &self.status_banner {
                                ui.label(msg);
                            }
                            ui.add_space(right_inset);
                        });
                        ui.add_space(6.0);
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

        if self.fig.ui.home {
            self.show_home_drive_strip(ui);
        }

        self.tablet_input.canvas_rect = Some(ui.available_rect_before_wrap());

        // Render the real FigCraft UI (toolbars, Layers, Design/Inspect panels, Canvas)
        figcraft_ui_egui::windows::guard(&mut self.fig, &ctx, |fig| fig.ui(ui));

        // Install bundled/custom Korean UI font AFTER FigApp initializes its own icon fonts
        self.ensure_default_korean_ui_font(&ctx);
        if let Some(font_path) = self.pending_ui_font_path.take() {
            let _ = self.install_ui_font_from_path(&ctx, &font_path, "picked");
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
            self.restore_focus_before_paste = true;
            self.services.prompt_text_input("");
        }

        let wants_kb_now = ctx.egui_wants_keyboard_input() || ctx.output(|o| o.ime.is_some());
        if wants_kb_now != self.last_wants_keyboard {
            self.last_wants_keyboard = wants_kb_now;
            self.game_ime_active = wants_kb_now;
            self.game_ime_prev_committed = " ".to_owned();
            self.game_ime_last_composing.clear();
            platform::game_show_soft_input(wants_kb_now);
        }

        self.show_toolbar_editor_window(&ctx);
        self.show_mouse_settings_window(&ctx);
        self.poll_ngrok_tunnel();
        self.show_drive_window(&ctx);
        self.show_mcp_tunnel_window(&ctx);
        self.show_font_browser_window(&ctx);
        let agent_actions = self.agent_bubble.show(
            &ctx,
            &mut self.prefs,
            self.tablet_input.top_bar_bottom_pt,
            self.tablet_input.canvas_rect,
            self.mcp_server.is_some(),
        );
        if agent_actions.undo {
            self.fig.run("edit.undo", json!({}));
        }
        if agent_actions.prefs_dirty {
            self.services.save_prefs(&self.prefs);
        }
        if agent_actions.open_mcp_tunnel {
            self.mcp_window_open = true;
            self.mcp_window_anchor = None;
            self.mcp_window_open_seq = self.mcp_window_open_seq.wrapping_add(1);
        }
        self.tablet_input.paint_virtual_cursor(&ctx);

        if self.tablet_input.needs_repaint()
            || self.pending_paste_text.is_some()
            || !self.pending_egui_events.is_empty()
            || self.game_ime_active
            || self.mcp_server.is_some()
            || self.agent_bubble.needs_repaint()
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(40));
        }
    }
}
