//! Android platform services for FigCraft (V6_hotfix2).

use crate::platform;
use crate::protocol::{BridgeReply, SEP};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AndroidShellPrefs {
    pub ui_zoom_factor: f32,
    pub immersive_mode: bool,
}

impl Default for AndroidShellPrefs {
    fn default() -> Self {
        Self {
            ui_zoom_factor: 1.25,
            immersive_mode: false,
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
        prefs.ui_zoom_factor = prefs.ui_zoom_factor.clamp(0.75, 2.0);
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
