//! Android platform services for FigCraft (V6_hotfix2).

use crate::platform;
use crate::protocol::{BridgeReply, SEP};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolbarButtonId {
    Open,
    Image,
    Copy,
    Cut,
    Paste,
    Duplicate,
    Delete,
    MultiSelect,
    Undo,
    Redo,
    Group,
    ZoomFit,
    Mouse,
    UiZoom,
    Fullscreen,
    Fonts,
    UiFont,
    Mcp,
    CopyToken,
    Pad,
}

impl ToolbarButtonId {
    pub const ALL: &[Self] = &[
        Self::Open,
        Self::Image,
        Self::Copy,
        Self::Cut,
        Self::Paste,
        Self::Duplicate,
        Self::Delete,
        Self::MultiSelect,
        Self::Undo,
        Self::Redo,
        Self::Group,
        Self::ZoomFit,
        Self::Mouse,
        Self::Fullscreen,
        Self::Fonts,
        Self::UiFont,
        Self::Mcp,
        Self::CopyToken,
        Self::Pad,
    ];

    pub fn default_bar() -> Vec<Self> {
        vec![
            Self::Open,
            Self::Image,
            Self::MultiSelect,
            Self::Copy,
            Self::Cut,
            Self::Paste,
            Self::Duplicate,
            Self::Undo,
            Self::Redo,
            Self::Delete,
            Self::Mouse,
                Self::Fullscreen,
            Self::Pad,
        ]
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Open => "열기",
            Self::Image => "이미지",
            Self::Copy => "복사",
            Self::Cut => "잘라내기",
            Self::Paste => "붙여넣기",
            Self::Duplicate => "복제",
            Self::Delete => "삭제",
            Self::MultiSelect => "다중선택",
            Self::Undo => "실행취소",
            Self::Redo => "다시실행",
            Self::Group => "그룹",
            Self::ZoomFit => "화면맞춤",
            Self::Mouse => "마우스",
            Self::UiZoom => "배율",
            Self::Fullscreen => "전체화면",
            Self::Fonts => "폰트",
            Self::UiFont => "앱 폰트",
            Self::Mcp => "MCP",
            Self::CopyToken => "토큰 복사",
            Self::Pad => "단축키",
        }
    }
}

fn default_mouse_cursor_scale() -> f32 {
    1.0
}

fn default_mouse_grip_distance() -> f32 {
    52.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AndroidShellPrefs {
    pub ui_zoom_factor: f32,
    pub immersive_mode: bool,
    #[serde(default = "ToolbarButtonId::default_bar")]
    pub toolbar_buttons: Vec<ToolbarButtonId>,
    #[serde(default = "default_mouse_cursor_scale")]
    pub mouse_cursor_scale: f32,
    #[serde(default = "default_mouse_grip_distance")]
    pub mouse_grip_distance: f32,
}

impl Default for AndroidShellPrefs {
    fn default() -> Self {
        Self {
            ui_zoom_factor: 1.0,
            immersive_mode: false,
            toolbar_buttons: ToolbarButtonId::default_bar(),
            mouse_cursor_scale: default_mouse_cursor_scale(),
            mouse_grip_distance: default_mouse_grip_distance(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AndroidServices {
    pub internal_dir: PathBuf,
    pub recovery_dir: PathBuf,
    pub staged_fonts_dir: PathBuf,
    pub staging_dir: PathBuf,
    prefs_path: PathBuf,
}

impl AndroidServices {
    pub fn new(internal_dir: PathBuf) -> Self {
        let recovery_dir = internal_dir.join("recovery");
        let staged_fonts_dir = internal_dir.join("fonts");
        let staging_dir = internal_dir.join("staging");
        let prefs_path = internal_dir.join("android_shell_prefs.json");
        let _ = fs::create_dir_all(&recovery_dir);
        let _ = fs::create_dir_all(&staged_fonts_dir);
        let _ = fs::create_dir_all(&staging_dir);
        Self {
            internal_dir,
            recovery_dir,
            staged_fonts_dir,
            staging_dir,
            prefs_path,
        }
    }

    pub fn load_prefs(&self) -> AndroidShellPrefs {
        let mut prefs: AndroidShellPrefs = fs::read_to_string(&self.prefs_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        prefs.ui_zoom_factor = 1.0;
        prefs.toolbar_buttons.retain(|b| *b != ToolbarButtonId::UiZoom);
        prefs.mouse_cursor_scale = prefs.mouse_cursor_scale.clamp(0.6, 2.0);
        prefs.mouse_grip_distance = prefs.mouse_grip_distance.clamp(24.0, 120.0);
        prefs
    }

    pub fn save_prefs(&self, prefs: &AndroidShellPrefs) {
        if let Ok(json) = serde_json::to_string_pretty(prefs) {
            let _ = fs::write(&self.prefs_path, json);
        }
    }

    pub fn set_immersive_mode(&self, enable: bool) {
        let _ = platform::call_activity("set_immersive", if enable { "1" } else { "0" });
    }

    pub fn set_light_system_bars(&self, is_light: bool) {
        let _ = platform::call_activity("set_light_bars", if is_light { "1" } else { "0" });
    }

    pub fn request_open_file(&self) -> Result<(), String> {
        match platform::call_activity("open_file", "") {
            BridgeReply::Ok(_) => Ok(()),
            BridgeReply::Err(e) => Err(e),
        }
    }

    pub fn request_pick_image(&self) -> Result<(), String> {
        match platform::call_activity("pick_image", "") {
            BridgeReply::Ok(_) => Ok(()),
            BridgeReply::Err(e) => Err(e),
        }
    }

    pub fn request_pick_fonts_dir(&self) -> Result<(), String> {
        match platform::call_activity("pick_fonts_dir", "") {
            BridgeReply::Ok(_) => Ok(()),
            BridgeReply::Err(e) => Err(e),
        }
    }

    pub fn request_pick_ui_font(&self) -> Result<(), String> {
        match platform::call_activity("pick_ui_font", "") {
            BridgeReply::Ok(_) => Ok(()),
            BridgeReply::Err(e) => Err(e),
        }
    }

    pub fn get_local_ip(&self) -> Option<String> {
        match platform::call_activity("get_local_ip", "") {
            BridgeReply::Ok(Some(ip)) if !ip.is_empty() => Some(ip),
            _ => None,
        }
    }

    pub fn set_clipboard_text(&self, text: &str) {
        let _ = platform::call_activity("set_clipboard", text);
    }

    pub fn request_paste_clipboard(&self) -> Result<(), String> {
        match platform::call_activity("paste_clipboard", "") {
            BridgeReply::Ok(_) => Ok(()),
            BridgeReply::Err(e) => Err(e),
        }
    }

    pub fn save_via_mediastore(
        &self,
        subdir: &str,
        filename: &str,
        mime: &str,
        bytes: &[u8],
    ) -> Result<(), String> {
        let staged_path = self.staging_dir.join(filename);
        fs::write(&staged_path, bytes).map_err(|e| format!("stage write failed: {e}"))?;
        let arg = format!(
            "{subdir}{SEP}{filename}{SEP}{mime}{SEP}{}",
            staged_path.display()
        );
        let res = platform::call_activity("save_mediastore", &arg);
        let _ = fs::remove_file(&staged_path);
        match res {
            BridgeReply::Ok(_) => Ok(()),
            BridgeReply::Err(e) => Err(e),
        }
    }

    pub fn request_save_as(&self, suggested_name: &str, mime: &str) -> Result<(), String> {
        let arg = format!("{suggested_name}{SEP}{mime}");
        match platform::call_activity("save_as", &arg) {
            BridgeReply::Ok(_) => Ok(()),
            BridgeReply::Err(e) => Err(e),
        }
    }

    pub fn write_to_saf_uri(&self, uri: &str, filename: &str, bytes: &[u8]) -> Result<(), String> {
        let staged_path = self.staging_dir.join(filename);
        fs::write(&staged_path, bytes).map_err(|e| format!("stage write failed: {e}"))?;
        let arg = format!("{uri}{SEP}{}", staged_path.display());
        let res = platform::call_activity("write_uri", &arg);
        let _ = fs::remove_file(&staged_path);
        match res {
            BridgeReply::Ok(_) => Ok(()),
            BridgeReply::Err(e) => Err(e),
        }
    }

    pub fn set_soft_keyboard(&self, show: bool) {
        let _ = platform::call_activity("soft_keyboard", if show { "1" } else { "0" });
    }

    pub fn open_url(&self, url: &str) {
        let _ = platform::call_activity("open_url", url);
    }

    pub fn set_mcp_foreground_service(&self, running: bool, bind_label: &str) {
        let cmd = if running { "start_mcp_service" } else { "stop_mcp_service" };
        let _ = platform::call_activity(cmd, bind_label);
    }

    pub fn write_recovery_snapshot(&self, doc_id: &str, bytes: &[u8]) -> Result<(), String> {
        let tmp = self.recovery_dir.join(format!("{doc_id}.tmp"));
        let dst = self.recovery_dir.join(format!("{doc_id}.figcraft"));
        fs::write(&tmp, bytes).map_err(|e| format!("recovery tmp write failed: {e}"))?;
        fs::rename(&tmp, &dst).map_err(|e| format!("recovery rename failed: {e}"))?;
        Ok(())
    }

    pub fn read_recovery_snapshots(&self) -> Vec<Vec<u8>> {
        let mut entries: Vec<PathBuf> = fs::read_dir(&self.recovery_dir)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("figcraft"))
            .collect();
        entries.sort();
        entries
            .into_iter()
            .filter_map(|p| fs::read(p).ok())
            .filter(|b| !b.is_empty())
            .collect()
    }

    pub fn replace_recovery_snapshots(recovery_dir: &Path, snapshots: &[Vec<u8>]) {
        let _ = fs::create_dir_all(recovery_dir);
        if let Ok(read) = fs::read_dir(recovery_dir) {
            for entry in read.flatten() {
                let p = entry.path();
                if p.extension().and_then(|e| e.to_str()) == Some("figcraft") {
                    let _ = fs::remove_file(p);
                }
            }
        }
        for (idx, bytes) in snapshots.iter().enumerate() {
            let tmp = recovery_dir.join(format!("snap_{idx:03}.tmp"));
            let dst = recovery_dir.join(format!("snap_{idx:03}.figcraft"));
            if fs::write(&tmp, bytes).is_ok() {
                let _ = fs::rename(&tmp, &dst);
            }
        }
    }

    pub fn font_search_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        let system_fonts = Path::new("/system/fonts");
        if system_fonts.is_dir() {
            dirs.push(system_fonts.to_path_buf());
        }
        if self.staged_fonts_dir.is_dir() {
            dirs.push(self.staged_fonts_dir.clone());
        }
        dirs
    }
}

#[cfg(target_os = "android")]
pub fn install_panic_hook(app: &android_activity::AndroidApp) {
    let log_dir = app.internal_data_path().unwrap_or_else(|| PathBuf::from("/data/local/tmp"));
    let log_path = log_dir.join("last-panic.txt");
    std::panic::set_hook(Box::new(move |info| {
        let msg = format!("PANIC: {info}\n");
        log::error!("{msg}");
        let _ = fs::write(&log_path, &msg);
        let _ = platform::call_activity("save_crash_log", &msg);
    }));
}


pub fn encode_base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((n >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(n & 63) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}
