//! Text-based JNI bridge protocol between Rust and `FigCraftActivity.java` (V6).

pub const SEP: char = '\u{001F}';

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeReply {
    Ok(Option<String>),
    Err(String),
}

impl BridgeReply {
    pub fn parse(raw: &str) -> Self {
        if raw == "ok" {
            return Self::Ok(None);
        }
        if let Some(payload) = raw.strip_prefix(&format!("ok{SEP}")) {
            return Self::Ok(Some(payload.to_owned()));
        }
        if let Some(msg) = raw.strip_prefix(&format!("err{SEP}")) {
            return Self::Err(msg.to_owned());
        }
        Self::Err(format!("malformed bridge reply: {raw}"))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ActivityEvent {
    FileOpened { path: String, display_name: String },
    ImagePicked { path: String, display_name: String, mime: String },
    ClipboardText(String),
    ClipboardImage { path: String, mime: String },
    SaveAsPicked { uri: String, display_name: String },
    FontsSynced { count: usize },
    WindowInsets {
        top: f32,
        right: f32,
        bottom: f32,
        left: f32,
    },
    ImeChanged { visible: bool, height: f32 },
    StylusSample {
        x: f32,
        y: f32,
        pressure: f32,
        tilt_x: f32,
        tilt_y: f32,
        is_eraser: bool,
        action: StylusAction,
    },
    McpServiceStopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StylusAction {
    Down,
    Move,
    Up,
    Hover,
}

impl ActivityEvent {
    pub fn parse(line: &str) -> Option<Self> {
        // Use split_once first so clipboard_text preserves any embedded content intact.
        if let Some(rest) = line.strip_prefix(&format!("clipboard_text{SEP}")) {
            return Some(Self::ClipboardText(rest.to_owned()));
        }

        let mut parts = line.split(SEP);
        let kind = parts.next()?;
        match kind {
            "file_opened" => Some(Self::FileOpened {
                path: parts.next()?.to_owned(),
                display_name: parts.next().unwrap_or("untitled.figcraft").to_owned(),
            }),
            "image_picked" => Some(Self::ImagePicked {
                path: parts.next()?.to_owned(),
                display_name: parts.next().unwrap_or("image.png").to_owned(),
                mime: parts.next().unwrap_or("image/png").to_owned(),
            }),
            "clipboard_image" => Some(Self::ClipboardImage {
                path: parts.next()?.to_owned(),
                mime: parts.next().unwrap_or("image/png").to_owned(),
            }),
            "save_as" => Some(Self::SaveAsPicked {
                uri: parts.next()?.to_owned(),
                display_name: parts.next().unwrap_or("design.figcraft").to_owned(),
            }),
            "fonts_synced" => Some(Self::FontsSynced {
                count: parts.next()?.parse().ok()?,
            }),
            "insets" => Some(Self::WindowInsets {
                top: parts.next()?.parse().ok()?,
                right: parts.next()?.parse().ok()?,
                bottom: parts.next()?.parse().ok()?,
                left: parts.next()?.parse().ok()?,
            }),
            "ime" => Some(Self::ImeChanged {
                visible: parts.next()? == "1",
                height: parts.next()?.parse().ok()?,
            }),
            "stylus" => {
                let action = match parts.next()? {
                    "down" => StylusAction::Down,
                    "move" => StylusAction::Move,
                    "up" => StylusAction::Up,
                    _ => StylusAction::Hover,
                };
                Some(Self::StylusSample {
                    action,
                    x: parts.next()?.parse().ok()?,
                    y: parts.next()?.parse().ok()?,
                    pressure: parts.next()?.parse().ok()?,
                    tilt_x: parts.next()?.parse().ok()?,
                    tilt_y: parts.next()?.parse().ok()?,
                    is_eraser: parts.next()? == "1",
                })
            }
            "mcp_stopped" => Some(Self::McpServiceStopped),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_v6_image_and_clipboard_events() {
        assert_eq!(
            ActivityEvent::parse(&format!("clipboard_text{SEP}hello\nworld")),
            Some(ActivityEvent::ClipboardText("hello\nworld".into()))
        );
        assert_eq!(
            ActivityEvent::parse(&format!("image_picked{SEP}/tmp/a.png{SEP}a.png{SEP}image/png")),
            Some(ActivityEvent::ImagePicked {
                path: "/tmp/a.png".into(),
                display_name: "a.png".into(),
                mime: "image/png".into(),
            })
        );
        assert_eq!(
            ActivityEvent::parse(&format!("clipboard_image{SEP}/tmp/c.img{SEP}image/webp")),
            Some(ActivityEvent::ClipboardImage {
                path: "/tmp/c.img".into(),
                mime: "image/webp".into(),
            })
        );
    }
}
