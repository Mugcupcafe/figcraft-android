//! Floating draggable circular AI Agent mascot with Muse-style bottom card,
//! 18 detailed Korean action states with custom colorful vector mini-icons,
//! dedicated AI canvas cursor, and created/modified element highlights (V27).

use crate::services::AndroidShellPrefs;
use egui::{Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use serde_json::Value;
use std::collections::VecDeque;
use std::f32::consts::{PI, TAU};
use std::time::{Duration, Instant};

const ICON_SIZE: usize = 160;
const AGENT_IDLE_RGBA: &[u8] = include_bytes!("../assets/agent_idle.rgba");
const AGENT_WORKING_RGBA: &[u8] = include_bytes!("../assets/agent_working.rgba");
const AGENT_DONE_RGBA: &[u8] = include_bytes!("../assets/agent_done.rgba");
const AGENT_PAUSED_RGBA: &[u8] = include_bytes!("../assets/agent_paused.rgba");

const MAX_LOG_ENTRIES: usize = 8;
const MAX_HIGHLIGHTS: usize = 6;
const WORKING_HOLD_SECS: f32 = 2.1;
const DONE_HOLD_SECS: f32 = 5.0;
const HIGHLIGHT_TTL_SECS: f32 = 3.4;
const CURSOR_HOLD_SECS: f32 = 4.2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentVisualState {
    Idle,
    Working,
    Done,
    Paused,
}

impl AgentVisualState {
    pub fn label_ko(self) -> &'static str {
        match self {
            Self::Idle => "평상시 (대기)",
            Self::Working => "작업 중",
            Self::Done => "완료",
            Self::Paused => "일시정지",
        }
    }

    pub fn accent_color(self) -> Color32 {
        match self {
            Self::Idle => Color32::from_rgb(132, 232, 198),
            Self::Working => Color32::from_rgb(64, 224, 255),
            Self::Done => Color32::from_rgb(72, 224, 134),
            Self::Paused => Color32::from_rgb(245, 194, 84),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionCategory {
    Frame,
    Shape,
    Text,
    Typography,
    ColorFill,
    Stroke,
    Effect,
    AutoLayout,
    Transform,
    Vector,
    Boolean,
    Image,
    Component,
    Variable,
    Prototype,
    Cleanup,
    Inspect,
    ExportSave,
    Done,
    Paused,
    Generic,
}

impl ActionCategory {
    pub fn from_command(cmd: &str, params: Option<&Value>) -> Self {
        let c = cmd.trim();
        let node_type = params
            .and_then(|p| p.get("type"))
            .and_then(Value::as_str)
            .unwrap_or("");

        if c.starts_with("frame.") || c.starts_with("section.") || (c == "create_node" && (node_type == "frame" || node_type == "section")) {
            Self::Frame
        } else if c.starts_with("shape.") || c.starts_with("node.create") || c == "create_node" || c == "tool.place" {
            if node_type == "text" {
                Self::Text
            } else if node_type == "component" {
                Self::Component
            } else if node_type == "vector" {
                Self::Vector
            } else {
                Self::Shape
            }
        } else if c.starts_with("text.style")
            || c.starts_with("text.font")
            || c.starts_with("text.setVariations")
            || c.starts_with("text.list")
            || c.starts_with("text.rescan")
            || c.starts_with("text.refit")
        {
            Self::Typography
        } else if c.starts_with("text.") || c == "type_text" {
            Self::Text
        } else if c.starts_with("stroke.") {
            Self::Stroke
        } else if c.starts_with("effect.") {
            Self::Effect
        } else if c.starts_with("fill.") || c.starts_with("style.") || c.starts_with("node.set") || c == "set_properties" {
            Self::ColorFill
        } else if c.starts_with("layout.") {
            Self::AutoLayout
        } else if c.starts_with("node.move")
            || c.starts_with("node.resize")
            || c.starts_with("align.")
            || c.starts_with("distribute.")
            || c.starts_with("transform.")
            || c.starts_with("object.")
        {
            Self::Transform
        } else if c.starts_with("vector.") {
            Self::Vector
        } else if c.starts_with("bool.") || c.starts_with("shapeBuilder.") {
            Self::Boolean
        } else if c.starts_with("image.") {
            Self::Image
        } else if c.starts_with("component.") || c.starts_with("instance.") || c.starts_with("library.") {
            Self::Component
        } else if c.starts_with("variable.") {
            Self::Variable
        } else if c.starts_with("proto.") {
            Self::Prototype
        } else if c.starts_with("node.delete")
            || c.starts_with("edit.")
            || c == "undo"
            || c == "redo"
        {
            Self::Cleanup
        } else if c.starts_with("file.")
            || c.starts_with("export")
            || c == "screenshot"
            || c.starts_with("template.")
            || c.starts_with("version.")
            || c.starts_with("branch.")
            || c == "save_file"
            || c == "open_file"
        {
            Self::ExportSave
        } else if c.starts_with("doc.")
            || c.starts_with("page.")
            || c.starts_with("node.get")
            || c.starts_with("node.list")
            || c.starts_with("inspect")
            || c.starts_with("select.")
            || c.starts_with("measure.")
            || c == "get_node"
            || c == "figcraft.status"
        {
            Self::Inspect
        } else {
            Self::Generic
        }
    }

    pub fn label_ko(self, raw_cmd: &str) -> String {
        match self {
            Self::Frame => "프레임 만드는 중".into(),
            Self::Shape => "도형 그리는 중".into(),
            Self::Text => "텍스트 작성 중".into(),
            Self::Typography => "글꼴 스타일 조정 중".into(),
            Self::ColorFill => "색상 입히는 중".into(),
            Self::Stroke => "테두리 다듬는 중".into(),
            Self::Effect => "이펙트 적용 중".into(),
            Self::AutoLayout => "오토 레이아웃 맞추는 중".into(),
            Self::Transform => "위치·크기 맞추는 중".into(),
            Self::Vector => "벡터 패스 다듬는 중".into(),
            Self::Boolean => "도형 결합하는 중".into(),
            Self::Image => "이미지 배치하는 중".into(),
            Self::Component => "컴포넌트 구성 중".into(),
            Self::Variable => "변수·토큰 연결 중".into(),
            Self::Prototype => "프로토타입 연결 중".into(),
            Self::Cleanup => "요소 정리하는 중".into(),
            Self::Inspect => "캔버스 구조 확인 중".into(),
            Self::ExportSave => "파일 저장·내보내는 중".into(),
            Self::Done => "작업 완료!".into(),
            Self::Paused => "일시정지됨".into(),
            Self::Generic => {
                let c = raw_cmd.trim();
                if c.is_empty() {
                    "AI 작업 중".into()
                } else {
                    format!("{c} 실행 중")
                }
            }
        }
    }

    /// Draws a crisp 14x14 colorful vector mini-icon (zero emoji-font dependency).
    pub fn paint_icon(self, painter: &egui::Painter, center: Pos2) {
        match self {
            Self::Frame => {
                let col = Color32::from_rgb(76, 224, 178);
                let r = Rect::from_center_size(center, Vec2::splat(9.0));
                painter.rect_stroke(r, CornerRadius::same(1), Stroke::new(1.5, col), StrokeKind::Middle);
                // Hash extensions like Figma frame icon
                painter.line_segment([Pos2::new(r.left() - 2.0, r.top()), Pos2::new(r.right() + 2.0, r.top())], Stroke::new(1.3, col));
                painter.line_segment([Pos2::new(r.left() - 2.0, r.bottom()), Pos2::new(r.right() + 2.0, r.bottom())], Stroke::new(1.3, col));
                painter.line_segment([Pos2::new(r.left(), r.top() - 2.0), Pos2::new(r.left(), r.bottom() + 2.0)], Stroke::new(1.3, col));
                painter.line_segment([Pos2::new(r.right(), r.top() - 2.0), Pos2::new(r.right(), r.bottom() + 2.0)], Stroke::new(1.3, col));
            }
            Self::Shape => {
                let r = Rect::from_center_size(center + Vec2::new(-1.8, 1.5), Vec2::splat(7.5));
                painter.rect_filled(r, CornerRadius::same(2), Color32::from_rgb(76, 196, 255));
                painter.circle_filled(center + Vec2::new(2.5, -2.0), 3.6, Color32::from_rgb(122, 238, 198));
            }
            Self::Text => {
                let col = Color32::from_rgb(86, 172, 255);
                painter.rect_filled(Rect::from_center_size(center, Vec2::splat(13.0)), CornerRadius::same(3), Color32::from_rgba_unmultiplied(86, 172, 255, 45));
                painter.text(center, egui::Align2::CENTER_CENTER, "T", FontId::proportional(11.5), col);
            }
            Self::Typography => {
                let col = Color32::from_rgb(186, 134, 255);
                painter.text(center, egui::Align2::CENTER_CENTER, "Aa", FontId::proportional(10.0), col);
            }
            Self::ColorFill => {
                painter.circle_filled(center + Vec2::new(-3.0, -2.0), 3.2, Color32::from_rgb(255, 114, 132));
                painter.circle_filled(center + Vec2::new(3.0, -2.0), 3.2, Color32::from_rgb(255, 212, 86));
                painter.circle_filled(center + Vec2::new(0.0, 2.8), 3.2, Color32::from_rgb(76, 214, 255));
            }
            Self::Stroke => {
                let col = Color32::from_rgb(255, 168, 76);
                painter.line_segment(
                    [center + Vec2::new(-4.8, 4.2), center + Vec2::new(4.8, -4.2)],
                    Stroke::new(2.4, col),
                );
                painter.circle_filled(center + Vec2::new(-4.8, 4.2), 2.0, Color32::WHITE);
                painter.circle_filled(center + Vec2::new(4.8, -4.2), 2.0, Color32::WHITE);
            }
            Self::Effect => {
                let col = Color32::from_rgb(255, 218, 86);
                // 4-point sparkle star
                let s = 6.0;
                let m = 2.0;
                let pts = [
                    center + Vec2::new(0.0, -s),
                    center + Vec2::new(m, -m),
                    center + Vec2::new(s, 0.0),
                    center + Vec2::new(m, m),
                    center + Vec2::new(0.0, s),
                    center + Vec2::new(-m, m),
                    center + Vec2::new(-s, 0.0),
                    center + Vec2::new(-m, -m),
                ];
                painter.add(egui::Shape::convex_polygon(vec![pts[0], pts[1], pts[3], pts[4], pts[5], pts[7]], col, Stroke::NONE));
                painter.add(egui::Shape::convex_polygon(vec![pts[2], pts[3], pts[5], pts[6], pts[7], pts[1]], col, Stroke::NONE));
            }
            Self::AutoLayout => {
                let col = Color32::from_rgb(84, 228, 146);
                for dx in [-4.0_f32, 0.0, 4.0] {
                    let bar = Rect::from_center_size(center + Vec2::new(dx, 0.0), Vec2::new(2.5, 10.0));
                    painter.rect_filled(bar, CornerRadius::same(1), col);
                }
            }
            Self::Transform => {
                let col = Color32::from_rgb(82, 232, 206);
                painter.line_segment([center + Vec2::new(-5.0, 0.0), center + Vec2::new(5.0, 0.0)], Stroke::new(1.7, col));
                painter.line_segment([center + Vec2::new(0.0, -5.0), center + Vec2::new(0.0, 5.0)], Stroke::new(1.7, col));
                painter.circle_filled(center, 2.0, col);
            }
            Self::Vector => {
                let col = Color32::from_rgb(255, 152, 82);
                let tip = center + Vec2::new(4.5, -4.5);
                let a = center + Vec2::new(-4.5, 4.5);
                let b = center + Vec2::new(-1.5, -2.5);
                let c = center + Vec2::new(2.5, 1.5);
                painter.add(egui::Shape::convex_polygon(vec![tip, b, a, c], col, Stroke::NONE));
                painter.circle_filled(center, 1.4, Color32::from_rgb(24, 28, 34));
            }
            Self::Boolean => {
                let c1 = Color32::from_rgba_unmultiplied(180, 128, 255, 220);
                let c2 = Color32::from_rgba_unmultiplied(92, 212, 255, 220);
                painter.circle_filled(center + Vec2::new(-2.4, 0.0), 4.0, c1);
                painter.circle_filled(center + Vec2::new(2.4, 0.0), 4.0, c2);
            }
            Self::Image => {
                let frame = Rect::from_center_size(center, Vec2::new(12.0, 10.0));
                painter.rect_filled(frame, CornerRadius::same(2), Color32::from_rgb(248, 128, 178));
                painter.circle_filled(center + Vec2::new(2.8, -2.2), 1.5, Color32::from_rgb(255, 240, 150));
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        Pos2::new(frame.left() + 1.5, frame.bottom() - 1.0),
                        Pos2::new(center.x - 1.0, center.y - 0.5),
                        Pos2::new(frame.right() - 1.5, frame.bottom() - 1.0),
                    ],
                    Color32::from_rgb(255, 230, 240),
                    Stroke::NONE,
                ));
            }
            Self::Component => {
                let col = Color32::from_rgb(176, 124, 255);
                for (dx, dy) in [(0.0, -3.4), (3.4, 0.0), (0.0, 3.4), (-3.4, 0.0)] {
                    let c = center + Vec2::new(dx, dy);
                    let r = 2.1;
                    painter.add(egui::Shape::convex_polygon(
                        vec![
                            c + Vec2::new(0.0, -r),
                            c + Vec2::new(r, 0.0),
                            c + Vec2::new(0.0, r),
                            c + Vec2::new(-r, 0.0),
                        ],
                        col,
                        Stroke::NONE,
                    ));
                }
            }
            Self::Variable => {
                let col = Color32::from_rgb(76, 218, 242);
                painter.line_segment([center + Vec2::new(-5.0, -2.6), center + Vec2::new(5.0, -2.6)], Stroke::new(1.5, col));
                painter.circle_filled(center + Vec2::new(-1.8, -2.6), 2.2, col);
                painter.line_segment([center + Vec2::new(-5.0, 2.6), center + Vec2::new(5.0, 2.6)], Stroke::new(1.5, col));
                painter.circle_filled(center + Vec2::new(2.0, 2.6), 2.2, col);
            }
            Self::Prototype => {
                let col = Color32::from_rgb(86, 176, 255);
                painter.circle_stroke(center + Vec2::new(-3.6, 0.0), 2.4, Stroke::new(1.6, col));
                painter.line_segment([center + Vec2::new(-1.0, 0.0), center + Vec2::new(4.8, 0.0)], Stroke::new(1.7, col));
                painter.line_segment([center + Vec2::new(2.2, -2.6), center + Vec2::new(4.8, 0.0)], Stroke::new(1.7, col));
                painter.line_segment([center + Vec2::new(2.2, 2.6), center + Vec2::new(4.8, 0.0)], Stroke::new(1.7, col));
            }
            Self::Cleanup => {
                let col = Color32::from_rgb(255, 126, 126);
                let body = Rect::from_center_size(center + Vec2::new(0.0, 1.4), Vec2::new(8.0, 7.6));
                painter.rect_filled(body, CornerRadius::same(1), col);
                painter.line_segment([center + Vec2::new(-5.2, -3.2), center + Vec2::new(5.2, -3.2)], Stroke::new(1.8, col));
                painter.line_segment([center + Vec2::new(-2.0, -5.0), center + Vec2::new(2.0, -5.0)], Stroke::new(1.6, col));
            }
            Self::Inspect => {
                let col = Color32::from_rgb(138, 206, 255);
                painter.circle_stroke(center + Vec2::new(-1.4, -1.4), 4.0, Stroke::new(1.8, col));
                painter.line_segment([center + Vec2::new(1.6, 1.6), center + Vec2::new(5.2, 5.2)], Stroke::new(2.0, col));
            }
            Self::ExportSave => {
                let col = Color32::from_rgb(82, 226, 146);
                painter.line_segment([center + Vec2::new(0.0, -5.0), center + Vec2::new(0.0, 2.5)], Stroke::new(1.8, col));
                painter.line_segment([center + Vec2::new(-3.0, -0.4), center + Vec2::new(0.0, 2.5)], Stroke::new(1.8, col));
                painter.line_segment([center + Vec2::new(3.0, -0.4), center + Vec2::new(0.0, 2.5)], Stroke::new(1.8, col));
                painter.line_segment([center + Vec2::new(-4.8, 4.8), center + Vec2::new(4.8, 4.8)], Stroke::new(1.8, col));
            }
            Self::Done => {
                let col = Color32::from_rgb(72, 224, 134);
                painter.circle_filled(center, 6.2, col);
                painter.line_segment([center + Vec2::new(-2.8, 0.2), center + Vec2::new(-0.6, 2.5)], Stroke::new(1.8, Color32::from_rgb(14, 24, 20)));
                painter.line_segment([center + Vec2::new(-0.6, 2.5), center + Vec2::new(3.2, -2.2)], Stroke::new(1.8, Color32::from_rgb(14, 24, 20)));
            }
            Self::Paused => {
                let col = Color32::from_rgb(245, 194, 84);
                painter.circle_filled(center, 6.2, col);
                painter.line_segment([center + Vec2::new(-1.8, -2.6), center + Vec2::new(-1.8, 2.6)], Stroke::new(2.0, Color32::from_rgb(24, 22, 16)));
                painter.line_segment([center + Vec2::new(1.8, -2.6), center + Vec2::new(1.8, 2.6)], Stroke::new(2.0, Color32::from_rgb(24, 22, 16)));
            }
            Self::Generic => {
                painter.circle_filled(center, 4.5, Color32::from_rgb(98, 236, 204));
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighlightKind {
    Created,
    Modified,
    Deleted,
}

impl HighlightKind {
    fn tag_ko(self) -> &'static str {
        match self {
            Self::Created => "생성",
            Self::Modified => "수정",
            Self::Deleted => "삭제",
        }
    }

    fn color(self) -> Color32 {
        match self {
            Self::Created => Color32::from_rgb(72, 232, 168),
            Self::Modified => Color32::from_rgb(64, 212, 255),
            Self::Deleted => Color32::from_rgb(255, 118, 118),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ElementHighlight {
    pub rect: Rect,
    pub label: String,
    pub kind: HighlightKind,
    pub created_at: Instant,
}

#[derive(Debug, Clone)]
pub struct AgentLogEntry {
    pub summary: String,
    pub raw_cmd: String,
    pub category: ActionCategory,
    pub ok: bool,
    pub at: Instant,
}

#[derive(Debug, Default)]
pub struct AgentBubbleActions {
    pub undo: bool,
    pub prefs_dirty: bool,
    pub open_mcp_tunnel: bool,
}

struct AgentTextures {
    idle: egui::TextureHandle,
    working: egui::TextureHandle,
    done: egui::TextureHandle,
    paused: egui::TextureHandle,
}

struct DemoStep {
    rel_rect: [f32; 4],
    label: &'static str,
    category: ActionCategory,
    kind: HighlightKind,
}

const DEMO_STEPS: &[DemoStep] = &[
    DemoStep {
        rel_rect: [0.16, 0.20, 0.30, 0.24],
        label: "Hero Card",
        category: ActionCategory::Frame,
        kind: HighlightKind::Created,
    },
    DemoStep {
        rel_rect: [0.20, 0.25, 0.22, 0.06],
        label: "헤더 제목",
        category: ActionCategory::Text,
        kind: HighlightKind::Modified,
    },
    DemoStep {
        rel_rect: [0.52, 0.24, 0.24, 0.16],
        label: "그라데이션 배경",
        category: ActionCategory::ColorFill,
        kind: HighlightKind::Modified,
    },
    DemoStep {
        rel_rect: [0.52, 0.46, 0.24, 0.14],
        label: "버튼 리스트",
        category: ActionCategory::AutoLayout,
        kind: HighlightKind::Created,
    },
];

pub struct AgentBubbleState {
    textures: Option<AgentTextures>,
    pub paused: bool,
    detected_client_name: Option<String>,
    preview_override: Option<(AgentVisualState, Instant)>,
    last_work_at: Option<Instant>,
    done_entered_at: Option<Instant>,
    last_visual_state: AgentVisualState,
    burst_count: u32,
    current_task_text: String,
    current_category: ActionCategory,
    last_done_text: String,
    logs: VecDeque<AgentLogEntry>,
    pub popover_open: bool,
    popover_open_seq: u64,
    dragging: bool,
    drag_moved: bool,
    drag_origin: Option<Pos2>,
    drag_start_bubble_pos: Option<Pos2>,
    bubble_pos: Option<Pos2>,

    // Dedicated AI canvas cursor & element highlights
    cursor_pos: Option<Pos2>,
    cursor_target: Option<Pos2>,
    cursor_label: String,
    cursor_active_until: Option<Instant>,
    last_frame_at: Option<Instant>,
    highlights: VecDeque<ElementHighlight>,
    demo_started_at: Option<Instant>,
    demo_step_index: usize,
}

impl Default for AgentBubbleState {
    fn default() -> Self {
        Self {
            textures: None,
            paused: false,
            detected_client_name: None,
            preview_override: None,
            last_work_at: None,
            done_entered_at: None,
            last_visual_state: AgentVisualState::Idle,
            burst_count: 0,
            current_task_text: String::new(),
            current_category: ActionCategory::Generic,
            last_done_text: String::new(),
            logs: VecDeque::with_capacity(MAX_LOG_ENTRIES),
            popover_open: false,
            popover_open_seq: 0,
            dragging: false,
            drag_moved: false,
            drag_origin: None,
            drag_start_bubble_pos: None,
            bubble_pos: None,
            cursor_pos: None,
            cursor_target: None,
            cursor_label: String::new(),
            cursor_active_until: None,
            last_frame_at: None,
            highlights: VecDeque::with_capacity(MAX_HIGHLIGHTS),
            demo_started_at: None,
            demo_step_index: 0,
        }
    }
}

impl AgentBubbleState {
    pub fn set_detected_client_name(&mut self, raw: &str) {
        let low = raw.trim().to_ascii_lowercase();
        if low.is_empty() {
            return;
        }
        let nice = if low.contains("gemini") {
            "Gemini".to_owned()
        } else if low.contains("claude") {
            "Claude".to_owned()
        } else if low.contains("chatgpt") || low.contains("openai") || low.contains("gpt") {
            "ChatGPT".to_owned()
        } else if low.contains("muse") {
            "Muse".to_owned()
        } else if low.contains("grok") {
            "Grok".to_owned()
        } else if low.contains("deepseek") {
            "DeepSeek".to_owned()
        } else if low.contains("cursor") {
            "Cursor".to_owned()
        } else {
            raw.trim().chars().take(18).collect()
        };
        self.detected_client_name = Some(nice);
    }

    pub fn display_agent_name(&self, prefs: &AndroidShellPrefs) -> String {
        let custom = prefs.ai_agent_name.trim();
        if !custom.is_empty() {
            return custom.to_owned();
        }
        if let Some(det) = &self.detected_client_name {
            return det.clone();
        }
        "Gemini".to_owned()
    }

    fn classify_kind(cmd: &str) -> HighlightKind {
        let c = cmd.trim();
        if c.contains("create")
            || c.starts_with("shape.")
            || c.starts_with("frame.")
            || c.starts_with("image.place")
            || c.starts_with("instance.create")
            || c.starts_with("component.create")
            || c.contains("duplicate")
        {
            HighlightKind::Created
        } else if c.contains("delete") || c.contains("remove") {
            HighlightKind::Deleted
        } else {
            HighlightKind::Modified
        }
    }

    fn extract_bounds_from_value(v: &Value) -> Option<(f32, f32, f32, f32, Option<String>)> {
        let name = v
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| {
                v.pointer("/node/name")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            });

        let candidates = [
            v.get("absoluteBounds"),
            v.pointer("/node/absoluteBounds"),
            v.get("bounds"),
            v.pointer("/node/bounds"),
            v.get("node"),
            Some(v),
        ];

        for cand in candidates.into_iter().flatten() {
            let x = cand.get("x").and_then(Value::as_f64);
            let y = cand.get("y").and_then(Value::as_f64);
            if let (Some(x), Some(y)) = (x, y) {
                let w = cand
                    .get("width")
                    .or_else(|| cand.get("w"))
                    .and_then(Value::as_f64)
                    .unwrap_or(120.0)
                    .max(16.0);
                let h = cand
                    .get("height")
                    .or_else(|| cand.get("h"))
                    .and_then(Value::as_f64)
                    .unwrap_or(48.0)
                    .max(16.0);
                return Some((x as f32, y as f32, w as f32, h as f32, name));
            }
        }
        None
    }

    pub fn record_activity_with_target(
        &mut self,
        raw_cmd: &str,
        ok: bool,
        params: Option<&Value>,
        output: Option<&Value>,
        canvas_rect: Option<Rect>,
    ) {
        let now = Instant::now();
        let category = ActionCategory::from_command(raw_cmd, params);
        let summary = category.label_ko(raw_cmd);
        let within_burst = self
            .last_work_at
            .map(|t| now.duration_since(t).as_secs_f32() < WORKING_HOLD_SECS + 1.5)
            .unwrap_or(false);
        if within_burst {
            self.burst_count = self.burst_count.saturating_add(1);
        } else {
            self.burst_count = 1;
        }
        self.last_work_at = Some(now);
        self.preview_override = None;
        self.demo_started_at = None;
        self.current_task_text = summary.clone();
        self.current_category = category;
        self.last_done_text = if self.burst_count > 1 {
            format!("완료 · {}개 작업", self.burst_count)
        } else {
            "작업 완료!".into()
        };

        if self.logs.len() >= MAX_LOG_ENTRIES {
            self.logs.pop_back();
        }
        self.logs.push_front(AgentLogEntry {
            summary: summary.clone(),
            raw_cmd: raw_cmd.to_owned(),
            category,
            ok,
            at: now,
        });

        if ok && raw_cmd != "figcraft.status" {
            let bounds = output
                .and_then(Self::extract_bounds_from_value)
                .or_else(|| params.and_then(Self::extract_bounds_from_value));
            let kind = Self::classify_kind(raw_cmd);

            if let Some(canvas) = canvas_rect {
                let (screen_rect, elem_name) = if let Some((px, py, pw, ph, name)) = bounds {
                    let origin = canvas.left_top() + Vec2::new(60.0, 60.0);
                    let raw_min = origin + Vec2::new(px, py);
                    let clamped_w = pw.clamp(28.0, (canvas.width() * 0.72).max(40.0));
                    let clamped_h = ph.clamp(24.0, (canvas.height() * 0.68).max(32.0));
                    let min_x = raw_min.x.clamp(
                        canvas.left() + 18.0,
                        (canvas.right() - clamped_w - 18.0).max(canvas.left() + 18.0),
                    );
                    let min_y = raw_min.y.clamp(
                        canvas.top() + 24.0,
                        (canvas.bottom() - clamped_h - 24.0).max(canvas.top() + 24.0),
                    );
                    (
                        Rect::from_min_size(Pos2::new(min_x, min_y), Vec2::new(clamped_w, clamped_h)),
                        name,
                    )
                } else {
                    let hash = raw_cmd
                        .bytes()
                        .fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
                    let ox = ((hash % 9) as f32 - 4.0) * 26.0;
                    let oy = (((hash / 9) % 7) as f32 - 3.0) * 20.0;
                    let c = canvas.center() + Vec2::new(ox, oy);
                    (Rect::from_center_size(c, Vec2::new(150.0, 74.0)), None)
                };

                let label = match elem_name {
                    Some(n) if !n.trim().is_empty() => format!("{} · {}", kind.tag_ko(), n.trim()),
                    _ => kind.tag_ko().to_owned(),
                };
                self.push_highlight(screen_rect, label, kind, now);
                self.set_cursor_target(
                    Pos2::new(
                        screen_rect.left() + screen_rect.width() * 0.25,
                        screen_rect.top() + screen_rect.height() * 0.35,
                    ),
                    summary,
                    now,
                );
            }
        }
    }

    fn push_highlight(&mut self, rect: Rect, label: String, kind: HighlightKind, now: Instant) {
        if self.highlights.len() >= MAX_HIGHLIGHTS {
            self.highlights.pop_back();
        }
        self.highlights.push_front(ElementHighlight {
            rect,
            label,
            kind,
            created_at: now,
        });
    }

    fn set_cursor_target(&mut self, target: Pos2, label: String, now: Instant) {
        if self.cursor_pos.is_none() {
            let start = self.bubble_pos.unwrap_or(target + Vec2::new(40.0, 40.0));
            self.cursor_pos = Some(start);
        }
        self.cursor_target = Some(target);
        self.cursor_label = label;
        self.cursor_active_until = Some(now + Duration::from_secs_f32(CURSOR_HOLD_SECS));
    }

    fn start_cursor_demo(&mut self) {
        self.demo_started_at = Some(Instant::now());
        self.demo_step_index = 0;
        self.paused = false;
        self.preview_override = None;
        self.highlights.clear();
        self.popover_open = false;
    }

    fn tick_demo(&mut self, canvas_rect: Rect, now: Instant) {
        let Some(started) = self.demo_started_at else {
            return;
        };
        let elapsed = now.duration_since(started).as_secs_f32();
        let step_interval = 1.35;
        let desired_idx = (elapsed / step_interval) as usize;
        if desired_idx < DEMO_STEPS.len() && desired_idx >= self.demo_step_index {
            let step = &DEMO_STEPS[desired_idx];
            let r = Rect::from_min_size(
                Pos2::new(
                    canvas_rect.left() + canvas_rect.width() * step.rel_rect[0],
                    canvas_rect.top() + canvas_rect.height() * step.rel_rect[1],
                ),
                Vec2::new(
                    canvas_rect.width() * step.rel_rect[2],
                    canvas_rect.height() * step.rel_rect[3],
                ),
            );
            let act_text = step.category.label_ko("");
            self.push_highlight(
                r,
                format!("{} · {}", step.kind.tag_ko(), step.label),
                step.kind,
                now,
            );
            self.set_cursor_target(
                Pos2::new(r.left() + r.width() * 0.22, r.top() + r.height() * 0.35),
                act_text.clone(),
                now,
            );
            self.last_work_at = Some(now);
            self.current_task_text = act_text;
            self.current_category = step.category;
            self.burst_count = (desired_idx + 1) as u32;
            self.last_done_text = format!("완료 · {}개 작업", self.burst_count);
            self.demo_step_index = desired_idx + 1;
        } else if elapsed > (DEMO_STEPS.len() as f32) * step_interval + 0.6 {
            self.demo_started_at = None;
        }
    }

    pub fn current_state(&mut self) -> AgentVisualState {
        let now = Instant::now();
        let st = if let Some((ov, until)) = self.preview_override {
            if now < until {
                ov
            } else {
                self.preview_override = None;
                AgentVisualState::Idle
            }
        } else if self.paused {
            AgentVisualState::Paused
        } else if let Some(last) = self.last_work_at {
            let elapsed = now.duration_since(last).as_secs_f32();
            if elapsed < WORKING_HOLD_SECS {
                AgentVisualState::Working
            } else if elapsed < WORKING_HOLD_SECS + DONE_HOLD_SECS {
                AgentVisualState::Done
            } else {
                AgentVisualState::Idle
            }
        } else {
            AgentVisualState::Idle
        };

        if st == AgentVisualState::Done && self.last_visual_state != AgentVisualState::Done {
            self.done_entered_at = Some(now);
        }
        self.last_visual_state = st;
        st
    }

    pub fn needs_repaint(&mut self) -> bool {
        let now = Instant::now();
        let cursor_alive = self.cursor_active_until.map(|u| now < u).unwrap_or(false);
        let highlights_alive = !self.highlights.is_empty();
        matches!(
            self.current_state(),
            AgentVisualState::Working | AgentVisualState::Done | AgentVisualState::Paused
        ) || self.preview_override.is_some()
            || self.demo_started_at.is_some()
            || cursor_alive
            || highlights_alive
    }

    fn ensure_textures(&mut self, ctx: &egui::Context) {
        if self.textures.is_some() {
            return;
        }
        let load = |name: &str, bytes: &[u8]| {
            let img = egui::ColorImage::from_rgba_unmultiplied([ICON_SIZE, ICON_SIZE], bytes);
            ctx.load_texture(name, img, egui::TextureOptions::LINEAR)
        };
        self.textures = Some(AgentTextures {
            idle: load("agent_mochi_idle", AGENT_IDLE_RGBA),
            working: load("agent_mochi_working", AGENT_WORKING_RGBA),
            done: load("agent_mochi_done", AGENT_DONE_RGBA),
            paused: load("agent_mochi_paused", AGENT_PAUSED_RGBA),
        });
    }

    fn paint_canvas_cursor_and_highlights(
        &mut self,
        ctx: &egui::Context,
        prefs: &AndroidShellPrefs,
        canvas_rect: Rect,
        agent_name: &str,
        now: Instant,
        dt: f32,
    ) {
        if !prefs.ai_cursor_enabled {
            return;
        }

        self.highlights
            .retain(|h| now.duration_since(h.created_at).as_secs_f32() < HIGHLIGHT_TTL_SECS);

        if let (Some(cur), Some(target)) = (self.cursor_pos, self.cursor_target) {
            let alpha = 1.0 - (-10.5 * dt.clamp(0.001, 0.1)).exp();
            let next = Pos2::new(
                cur.x + (target.x - cur.x) * alpha,
                cur.y + (target.y - cur.y) * alpha,
            );
            self.cursor_pos = Some(next);
        }

        let cursor_remaining = self
            .cursor_active_until
            .map(|u| u.saturating_duration_since(now).as_secs_f32())
            .unwrap_or(0.0);

        if self.highlights.is_empty() && cursor_remaining <= 0.0 {
            self.cursor_pos = None;
            self.cursor_target = None;
            return;
        }

        let t = ctx.input(|i| i.time) as f32;
        egui::Area::new(egui::Id::unique("figcraft_ai_agent_canvas_fx"))
            .order(egui::Order::Foreground)
            .fixed_pos(canvas_rect.left_top())
            .interactable(false)
            .show(ctx, |ui| {
                let painter = ui.painter();

                for hl in &self.highlights {
                    let age = now.duration_since(hl.created_at).as_secs_f32();
                    let fade_in = (age / 0.18).clamp(0.0, 1.0);
                    let fade_out = ((HIGHLIGHT_TTL_SECS - age) / 0.7).clamp(0.0, 1.0);
                    let alpha = fade_in * fade_out;
                    let base = hl.kind.color();
                    let pulse = (t * 6.0).sin() * 0.5 + 0.5;

                    let fill = Color32::from_rgba_unmultiplied(
                        base.r(),
                        base.g(),
                        base.b(),
                        (26.0 * alpha) as u8,
                    );
                    let glow = Color32::from_rgba_unmultiplied(
                        base.r(),
                        base.g(),
                        base.b(),
                        ((65.0 + pulse * 55.0) * alpha) as u8,
                    );
                    let border = Color32::from_rgba_unmultiplied(
                        base.r(),
                        base.g(),
                        base.b(),
                        (235.0 * alpha) as u8,
                    );

                    painter.rect_filled(hl.rect, CornerRadius::same(6), fill);
                    painter.rect_stroke(
                        hl.rect.expand(2.5),
                        CornerRadius::same(8),
                        Stroke::new(3.5, glow),
                        StrokeKind::Outside,
                    );
                    painter.rect_stroke(
                        hl.rect,
                        CornerRadius::same(6),
                        Stroke::new(1.8, border),
                        StrokeKind::Inside,
                    );

                    let galley = painter.layout_no_wrap(
                        hl.label.clone(),
                        FontId::proportional(11.0),
                        Color32::from_rgba_unmultiplied(15, 22, 26, (245.0 * alpha) as u8),
                    );
                    let badge_size = galley.size() + Vec2::new(12.0, 5.0);
                    let badge_min = Pos2::new(
                        hl.rect.left(),
                        (hl.rect.top() - badge_size.y - 4.0).max(canvas_rect.top() + 4.0),
                    );
                    let badge_rect = Rect::from_min_size(badge_min, badge_size);
                    painter.rect_filled(
                        badge_rect,
                        CornerRadius::same(6),
                        Color32::from_rgba_unmultiplied(
                            base.r(),
                            base.g(),
                            base.b(),
                            (235.0 * alpha) as u8,
                        ),
                    );
                    painter.galley(badge_min + Vec2::new(6.0, 2.5), galley, Color32::BLACK);
                }

                if cursor_remaining > 0.0 {
                    if let Some(tip) = self.cursor_pos {
                        let fade = (cursor_remaining / 0.6).clamp(0.0, 1.0);
                        let mint = Color32::from_rgba_unmultiplied(84, 238, 192, (250.0 * fade) as u8);
                        let outline = Color32::from_rgba_unmultiplied(18, 28, 32, (240.0 * fade) as u8);
                        let shadow = Color32::from_rgba_unmultiplied(0, 0, 0, (90.0 * fade) as u8);

                        let draw_arrow = |offset: Vec2, fill: Color32, stroke: Stroke| {
                            let p0 = tip + offset;
                            let p1 = tip + offset + Vec2::new(0.0, 19.0);
                            let p2 = tip + offset + Vec2::new(5.2, 14.2);
                            let p3 = tip + offset + Vec2::new(9.2, 21.0);
                            let p4 = tip + offset + Vec2::new(12.2, 19.4);
                            let p5 = tip + offset + Vec2::new(8.4, 12.6);
                            let p6 = tip + offset + Vec2::new(14.8, 12.2);
                            painter.add(egui::Shape::convex_polygon(vec![p0, p1, p2], fill, Stroke::NONE));
                            painter.add(egui::Shape::convex_polygon(vec![p0, p2, p5], fill, Stroke::NONE));
                            painter.add(egui::Shape::convex_polygon(vec![p0, p5, p6], fill, Stroke::NONE));
                            painter.add(egui::Shape::convex_polygon(vec![p2, p3, p4, p5], fill, Stroke::NONE));
                            if stroke.width > 0.0 {
                                painter.add(egui::Shape::closed_line(
                                    vec![p0, p1, p2, p3, p4, p5, p6],
                                    stroke,
                                ));
                            }
                        };
                        draw_arrow(Vec2::new(1.5, 2.0), shadow, Stroke::NONE);
                        draw_arrow(Vec2::ZERO, mint, Stroke::new(1.4, outline));

                        if !self.cursor_label.is_empty() {
                            let full_label = format!("{agent_name} · {}", self.cursor_label);
                            let galley = painter.layout_no_wrap(
                                full_label,
                                FontId::proportional(11.5),
                                Color32::from_rgba_unmultiplied(14, 24, 26, (250.0 * fade) as u8),
                            );
                            let pill_size = galley.size() + Vec2::new(14.0, 6.0);
                            let pill_min = Pos2::new(
                                (tip.x + 14.0).min(canvas_rect.right() - pill_size.x - 8.0),
                                (tip.y + 16.0).min(canvas_rect.bottom() - pill_size.y - 8.0),
                            );
                            let pill_rect = Rect::from_min_size(pill_min, pill_size);
                            painter.rect_filled(
                                pill_rect.translate(Vec2::new(0.0, 1.5)),
                                CornerRadius::same(10),
                                shadow,
                            );
                            painter.rect_filled(pill_rect, CornerRadius::same(10), mint);
                            painter.rect_stroke(
                                pill_rect,
                                CornerRadius::same(10),
                                Stroke::new(1.0, outline),
                                StrokeKind::Inside,
                            );
                            painter.galley(pill_min + Vec2::new(7.0, 3.0), galley, Color32::BLACK);
                        }
                    }
                }
            });
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        prefs: &mut AndroidShellPrefs,
        top_bar_bottom: f32,
        canvas_rect: Option<Rect>,
        mcp_active: bool,
    ) -> AgentBubbleActions {
        let mut actions = AgentBubbleActions::default();
        let now = Instant::now();
        let dt = self
            .last_frame_at
            .map(|prev| now.duration_since(prev).as_secs_f32())
            .unwrap_or(0.016);
        self.last_frame_at = Some(now);

        let screen = ctx.input(|i| i.viewport_rect());
        let eff_canvas = canvas_rect.unwrap_or_else(|| {
            Rect::from_min_max(
                Pos2::new(screen.left(), top_bar_bottom.max(screen.top() + 50.0)),
                screen.right_bottom(),
            )
        });

        let agent_name = self.display_agent_name(prefs);
        self.tick_demo(eff_canvas, now);
        self.paint_canvas_cursor_and_highlights(ctx, prefs, eff_canvas, &agent_name, now, dt);

        if !prefs.ai_bubble_visible {
            self.popover_open = false;
            return actions;
        }

        self.ensure_textures(ctx);
        let state = self.current_state();
        let size = prefs.ai_bubble_size.clamp(36.0, 96.0);
        let dark_mode = ctx.style().visuals.dark_mode;

        let min_x = screen.left() + 8.0;
        let max_x = (screen.right() - size - 8.0).max(min_x);
        let min_y = (top_bar_bottom + 10.0).max(screen.top() + 56.0);
        // Leave enough room at bottom for the centered name/status card under the circular icon
        let max_y = (screen.bottom() - size - 78.0).max(min_y);

        let initial_pos = if let Some(p) = self.bubble_pos {
            p
        } else if prefs.ai_bubble_pos_x >= 0.0 && prefs.ai_bubble_pos_y >= 0.0 {
            Pos2::new(prefs.ai_bubble_pos_x, prefs.ai_bubble_pos_y)
        } else {
            Pos2::new(max_x - 14.0, (max_y - 36.0).max(min_y))
        };
        let mut pos = Pos2::new(
            initial_pos.x.clamp(min_x, max_x),
            initial_pos.y.clamp(min_y, max_y),
        );
        self.bubble_pos = Some(pos);

        let t = ctx.input(|i| i.time) as f32;

        // Character micro-animations
        let (anim_dy, anim_scale) = match state {
            AgentVisualState::Working => {
                let dy = (t * 4.5).sin() * 3.0;
                let sc = 1.0 + (t * 4.5).cos() * 0.032;
                (dy, sc)
            }
            AgentVisualState::Done => {
                let age = self
                    .done_entered_at
                    .map(|d| now.duration_since(d).as_secs_f32())
                    .unwrap_or(10.0);
                if age < 0.48 {
                    let p = age / 0.48;
                    let bounce = (p * PI).sin() * (1.0 - p * 0.4);
                    (-bounce * 5.0, 1.0 + bounce * 0.13)
                } else {
                    (0.0, 1.0)
                }
            }
            _ => (0.0, 1.0),
        };

        // 1. Paint Muse-style bottom-center card UNDER the circular avatar (drawn first so avatar overlaps its top edge!)
        let status_row: Option<(ActionCategory, String)> = if !prefs.ai_bubble_show_speech || self.popover_open {
            None
        } else {
            match state {
                AgentVisualState::Working => {
                    let dots = match ((t * 2.8) as usize) % 3 {
                        0 => ".",
                        1 => "..",
                        _ => "...",
                    };
                    let base = if self.current_task_text.is_empty() {
                        "AI 작업 중"
                    } else {
                        self.current_task_text.as_str()
                    };
                    Some((self.current_category, format!("{base}{dots}")))
                }
                AgentVisualState::Done => Some((
                    ActionCategory::Done,
                    if self.last_done_text.is_empty() {
                        "작업 완료!".into()
                    } else {
                        self.last_done_text.clone()
                    },
                )),
                AgentVisualState::Paused => Some((ActionCategory::Paused, "일시정지됨".into())),
                AgentVisualState::Idle => None,
            }
        };

        if !self.popover_open {
            let bubble_cx = pos.x + size * 0.5;
            let card_bg = if dark_mode {
                Color32::from_rgba_unmultiplied(30, 34, 42, 242)
            } else {
                Color32::from_rgba_unmultiplied(255, 255, 255, 248)
            };
            let title_col = if dark_mode {
                Color32::from_rgb(245, 248, 252)
            } else {
                Color32::from_rgb(20, 24, 30)
            };
            let sub_col = if dark_mode {
                Color32::from_rgb(198, 208, 220)
            } else {
                Color32::from_rgb(92, 100, 112)
            };
            let card_stroke = if status_row.is_some() {
                Stroke::new(1.2, state.accent_color())
            } else if dark_mode {
                Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 32))
            } else {
                Stroke::new(1.0, Color32::from_rgba_unmultiplied(0, 0, 0, 22))
            };

            // Measure card size cleanly
            let (card_w, card_h, overlap_y) = if let Some((_, ref status_txt)) = status_row {
                let text_w = (status_txt.chars().count() as f32 * 9.5 + 48.0).clamp(138.0, 250.0);
                let name_w = (agent_name.chars().count() as f32 * 10.5 + 32.0).max(96.0);
                (text_w.max(name_w), 62.0, (size * 0.24).clamp(10.0, 20.0))
            } else {
                // Idle: compact name pill just below the circular avatar (like 17870.jpg)
                let name_w = (agent_name.chars().count() as f32 * 9.5 + 24.0).clamp(62.0, 148.0);
                (name_w, 26.0, 6.0)
            };

            let card_x = (bubble_cx - card_w * 0.5).clamp(
                screen.left() + 8.0,
                (screen.right() - card_w - 8.0).max(screen.left() + 8.0),
            );
            let card_y = (pos.y + size - overlap_y).min(screen.bottom() - card_h - 8.0);
            let card_rect = Rect::from_min_size(Pos2::new(card_x, card_y), Vec2::new(card_w, card_h));

            egui::Area::new(egui::Id::unique("figcraft_ai_agent_bottom_card"))
                .order(egui::Order::Foreground)
                .fixed_pos(card_rect.left_top())
                .interactable(false)
                .show(ctx, |ui| {
                    let painter = ui.painter();
                    let radius = if status_row.is_some() { 16 } else { 13 };
                    // Soft card shadow
                    painter.rect_filled(
                        card_rect.translate(Vec2::new(0.0, 3.0)),
                        CornerRadius::same(radius),
                        Color32::from_rgba_unmultiplied(0, 0, 0, 75),
                    );
                    painter.rect_filled(card_rect, CornerRadius::same(radius), card_bg);
                    painter.rect_stroke(card_rect, CornerRadius::same(radius), card_stroke, StrokeKind::Inside);

                    if let Some((cat, status_txt)) = &status_row {
                        // Line 1: Bold AI Name centered below overlap
                        let name_y = card_rect.top() + overlap_y * 0.65 + 12.0;
                        painter.text(
                            Pos2::new(card_rect.center().x, name_y),
                            egui::Align2::CENTER_CENTER,
                            &agent_name,
                            FontId::proportional(14.5),
                            title_col,
                        );
                        // Line 2: Colorful vector mini-icon + action status centered
                        let status_galley = painter.layout_no_wrap(
                            status_txt.clone(),
                            FontId::proportional(12.0),
                            sub_col,
                        );
                        let icon_w = 16.0;
                        let gap = 5.0;
                        let total_w = icon_w + gap + status_galley.size().x;
                        let start_x = card_rect.center().x - total_w * 0.5;
                        let row_y = card_rect.bottom() - 15.0;
                        cat.paint_icon(painter, Pos2::new(start_x + icon_w * 0.5, row_y));
                        painter.galley(
                            Pos2::new(start_x + icon_w + gap, row_y - status_galley.size().y * 0.5),
                            status_galley,
                            sub_col,
                        );
                    } else {
                        // Compact idle name pill
                        painter.text(
                            Pos2::new(card_rect.center().x, card_rect.center().y + 1.5),
                            egui::Align2::CENTER_CENTER,
                            &agent_name,
                            FontId::proportional(12.5),
                            title_col,
                        );
                    }
                });
        }

        // 2. Paint Floating Draggable Circular Avatar ON TOP of the bottom card (Muse overlap effect)
        let pad = 12.0;
        let area_pos = Pos2::new(pos.x - pad, pos.y - pad);
        egui::Area::new(egui::Id::unique("figcraft_ai_agent_bubble"))
            .order(egui::Order::Foreground)
            .fixed_pos(area_pos)
            .interactable(true)
            .show(ctx, |ui| {
                let (alloc_rect, resp) = ui.allocate_exact_size(
                    Vec2::splat(size + pad * 2.0),
                    Sense::click_and_drag(),
                );
                let draw_center = alloc_rect.center() + Vec2::new(0.0, anim_dy);
                let draw_size = size * anim_scale;
                let icon_rect = Rect::from_center_size(draw_center, Vec2::splat(draw_size));

                let pointer_pos = ctx.input(|i| i.pointer.interact_pos());
                if resp.drag_started() {
                    self.dragging = true;
                    self.drag_moved = false;
                    self.drag_origin = pointer_pos;
                    self.drag_start_bubble_pos = Some(pos);
                }
                if resp.dragged() {
                    if let (Some(origin), Some(cur), Some(start_b)) =
                        (self.drag_origin, pointer_pos, self.drag_start_bubble_pos)
                    {
                        let offset = cur - origin;
                        if self.drag_moved || offset.length() >= 14.0 {
                            self.drag_moved = true;
                            pos.x = (start_b.x + offset.x).clamp(min_x, max_x);
                            pos.y = (start_b.y + offset.y).clamp(min_y, max_y);
                            self.bubble_pos = Some(pos);
                        }
                    } else {
                        let delta = resp.drag_delta();
                        if delta.length() >= 4.0 {
                            self.drag_moved = true;
                            pos.x = (pos.x + delta.x).clamp(min_x, max_x);
                            pos.y = (pos.y + delta.y).clamp(min_y, max_y);
                            self.bubble_pos = Some(pos);
                        }
                    }
                }
                let mut tapped = false;
                if resp.drag_stopped() {
                    if self.drag_moved {
                        prefs.ai_bubble_pos_x = pos.x;
                        prefs.ai_bubble_pos_y = pos.y;
                        actions.prefs_dirty = true;
                    } else {
                        tapped = true;
                    }
                    self.dragging = false;
                    self.drag_moved = false;
                    self.drag_origin = None;
                    self.drag_start_bubble_pos = None;
                } else if resp.clicked() {
                    tapped = true;
                }
                if tapped {
                    self.popover_open = !self.popover_open;
                    if self.popover_open {
                        self.popover_open_seq = self.popover_open_seq.wrapping_add(1);
                    }
                }

                let painter = ui.painter();
                let radius = draw_size * 0.5;

                // Soft drop shadow
                painter.circle_filled(
                    alloc_rect.center() + Vec2::new(0.0, 3.0),
                    size * 0.48,
                    Color32::from_rgba_unmultiplied(0, 0, 0, 95),
                );

                // Working: pulsing glow + rotating neon cyan-mint spinner arc
                if state == AgentVisualState::Working {
                    let pulse = (t * 4.5).sin() * 0.5 + 0.5;
                    let ring_r = radius + 2.5 + pulse * 3.0;
                    painter.circle_stroke(
                        draw_center,
                        ring_r,
                        Stroke::new(
                            1.8,
                            Color32::from_rgba_unmultiplied(64, 232, 255, (70.0 + pulse * 95.0) as u8),
                        ),
                    );

                    let start_ang = t * 4.2;
                    let arc_r = radius + 3.2;
                    let segs = 16;
                    let pts: Vec<Pos2> = (0..=segs)
                        .map(|i| {
                            let a = start_ang + (i as f32 / segs as f32) * (PI * 0.65);
                            Pos2::new(draw_center.x + a.cos() * arc_r, draw_center.y + a.sin() * arc_r)
                        })
                        .collect();
                    painter.add(egui::Shape::line(
                        pts,
                        Stroke::new(2.8, Color32::from_rgb(92, 245, 255)),
                    ));
                }

                if let Some(texs) = &self.textures {
                    let tex = match state {
                        AgentVisualState::Idle => &texs.idle,
                        AgentVisualState::Working => &texs.working,
                        AgentVisualState::Done => &texs.done,
                        AgentVisualState::Paused => &texs.paused,
                    };
                    painter.image(
                        tex.id(),
                        icon_rect,
                        Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }

                // Done: celebratory mint & gold confetti burst
                if state == AgentVisualState::Done {
                    if let Some(done_at) = self.done_entered_at {
                        let age = now.duration_since(done_at).as_secs_f32();
                        if age < 1.25 {
                            let p = age / 1.25;
                            let fade = (1.0 - p).clamp(0.0, 1.0);
                            let palette = [
                                Color32::from_rgb(96, 245, 190),
                                Color32::from_rgb(255, 218, 92),
                                Color32::from_rgb(80, 224, 255),
                                Color32::from_rgb(255, 165, 185),
                            ];
                            for i in 0..8 {
                                let ang = (i as f32 / 8.0) * TAU + 0.35;
                                let dist = radius * 0.85 + p * (radius * 0.72 + 12.0);
                                let pt = Pos2::new(
                                    draw_center.x + ang.cos() * dist,
                                    draw_center.y + ang.sin() * dist - p * 4.0,
                                );
                                let c = palette[i % palette.len()];
                                painter.circle_filled(
                                    pt,
                                    (3.2 * (1.0 - p * 0.5)).max(1.2),
                                    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (240.0 * fade) as u8),
                                );
                            }
                        }
                    }
                }

                // Paused: gently rising sleepy "z Z"
                if state == AgentVisualState::Paused {
                    let phase1 = (t * 0.55) % 1.0;
                    let phase2 = (t * 0.55 + 0.5) % 1.0;
                    for (ph, ch, sz) in [(phase1, "z", 10.5_f32), (phase2, "Z", 13.0_f32)] {
                        let a = (ph * PI).sin().clamp(0.0, 1.0);
                        let zp = Pos2::new(
                            draw_center.x + radius * 0.55 + (ph * 4.0).sin() * 3.0,
                            draw_center.y - radius * 0.55 - ph * 14.0,
                        );
                        painter.text(
                            zp,
                            egui::Align2::CENTER_CENTER,
                            ch,
                            FontId::proportional(sz),
                            Color32::from_rgba_unmultiplied(248, 210, 118, (220.0 * a) as u8),
                        );
                    }
                }

                if resp.hovered() || self.popover_open {
                    painter.circle_stroke(
                        draw_center,
                        radius + 1.0,
                        Stroke::new(1.8, state.accent_color()),
                    );
                }
            });

        // 3. Tap Popover Menu
        if self.popover_open {
            let win_w = 296.0;
            let win_h = 465.0;
            let on_right_half = (pos.x + size * 0.5) > screen.center().x;
            let raw_x = if on_right_half {
                pos.x - win_w - 10.0
            } else {
                pos.x + size + 10.0
            };
            let raw_y = pos.y - 30.0;
            let pop_x = raw_x.clamp(screen.left() + 8.0, (screen.right() - win_w - 8.0).max(8.0));
            let pop_y = raw_y.clamp(min_y, (screen.bottom() - win_h - 12.0).max(min_y));

            let mut open = self.popover_open;
            egui::Window::new("AI 에이전트")
                .order(egui::Order::Foreground)
                .id(egui::Id::unique(("figcraft_ai_agent_popover", self.popover_open_seq)))
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .fixed_pos(Pos2::new(pop_x, pop_y))
                .fixed_size(Vec2::new(win_w, 0.0))
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        let (dot_rect, _) =
                            ui.allocate_exact_size(Vec2::splat(9.0), Sense::hover());
                        ui.painter()
                            .circle_filled(dot_rect.center(), 4.5, state.accent_color());
                        ui.strong(format!("{} · {}", agent_name, state.label_ko()));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let mcp_txt = if mcp_active { "MCP 대기" } else { "MCP 꺼짐" };
                            ui.small(mcp_txt);
                        });
                    });

                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let pause_lbl = if self.paused { "재개" } else { "일시정지" };
                        if ui
                            .add(
                                egui::Button::new(pause_lbl)
                                    .selected(self.paused)
                                    .min_size(Vec2::new(88.0, 32.0)),
                            )
                            .clicked()
                        {
                            self.paused = !self.paused;
                            self.preview_override = None;
                        }
                        if ui
                            .add(egui::Button::new("실행취소").min_size(Vec2::new(88.0, 32.0)))
                            .clicked()
                        {
                            actions.undo = true;
                        }
                        if ui
                            .add(egui::Button::new("숨기기").min_size(Vec2::new(76.0, 32.0)))
                            .clicked()
                        {
                            prefs.ai_bubble_visible = false;
                            actions.prefs_dirty = true;
                        }
                    });
                    if ui
                        .add(egui::Button::new("MCP · HTTPS 터널 설정 열기").min_size(Vec2::new(266.0, 28.0)))
                        .clicked()
                    {
                        actions.open_mcp_tunnel = true;
                        self.popover_open = false;
                    }

                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(4.0);

                    // AI Name selector (Auto + popular AI presets + custom)
                    ui.strong("AI 이름 표시");
                    ui.horizontal_wrapped(|ui| {
                        for (lbl, val) in [
                            ("자동", ""),
                            ("Gemini", "Gemini"),
                            ("Claude", "Claude"),
                            ("ChatGPT", "ChatGPT"),
                            ("Muse", "Muse"),
                            ("Grok", "Grok"),
                            ("DeepSeek", "DeepSeek"),
                        ] {
                            let sel = prefs.ai_agent_name.trim() == val;
                            if ui.add(egui::Button::new(lbl).selected(sel)).clicked() {
                                prefs.ai_agent_name = val.to_owned();
                                actions.prefs_dirty = true;
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.small("직접 입력:");
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut prefs.ai_agent_name)
                                .hint_text("비우면 MCP 자동 인식")
                                .desired_width(180.0),
                        );
                        if resp.changed() {
                            actions.prefs_dirty = true;
                        }
                    });

                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(4.0);

                    ui.strong(format!("아이콘 크기 ({:.0}pt)", prefs.ai_bubble_size));
                    ui.horizontal(|ui| {
                        for (lbl, val) in [("작게", 44.0_f32), ("보통", 56.0), ("크게", 72.0), ("특대", 88.0)] {
                            let sel = (prefs.ai_bubble_size - val).abs() < 2.0;
                            if ui
                                .add(egui::Button::new(lbl).selected(sel).min_size(Vec2::new(60.0, 26.0)))
                                .clicked()
                            {
                                prefs.ai_bubble_size = val;
                                actions.prefs_dirty = true;
                            }
                        }
                    });
                    if ui
                        .add(
                            egui::Slider::new(&mut prefs.ai_bubble_size, 36.0..=96.0)
                                .step_by(2.0)
                                .suffix(" pt"),
                        )
                        .changed()
                    {
                        actions.prefs_dirty = true;
                    }

                    if ui
                        .checkbox(&mut prefs.ai_bubble_show_speech, "하단 작업 상태 카드 표시")
                        .changed()
                    {
                        actions.prefs_dirty = true;
                    }
                    if ui
                        .checkbox(&mut prefs.ai_cursor_enabled, "AI 전용 커서 · 요소 하이라이트 표시")
                        .changed()
                    {
                        actions.prefs_dirty = true;
                    }

                    ui.add_space(4.0);
                    ui.separator();
                    ui.add_space(4.0);

                    ui.horizontal(|ui| {
                        ui.small("상태·커서 미리보기");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add(egui::Button::new("커서 데모 체험").min_size(Vec2::new(104.0, 24.0)))
                                .clicked()
                            {
                                self.start_cursor_demo();
                            }
                        });
                    });
                    ui.horizontal(|ui| {
                        for (lbl, st, cat, sample) in [
                            ("평상시", AgentVisualState::Idle, ActionCategory::Generic, ""),
                            ("작업중", AgentVisualState::Working, ActionCategory::Frame, "프레임 만드는 중"),
                            ("완료", AgentVisualState::Done, ActionCategory::Done, "완료 · 4개 작업"),
                            ("정지", AgentVisualState::Paused, ActionCategory::Paused, ""),
                        ] {
                            if ui
                                .add(egui::Button::new(lbl).min_size(Vec2::new(60.0, 26.0)))
                                .clicked()
                            {
                                self.preview_override =
                                    Some((st, Instant::now() + Duration::from_secs(4)));
                                if st == AgentVisualState::Working {
                                    self.current_task_text = sample.to_owned();
                                    self.current_category = cat;
                                } else if st == AgentVisualState::Done {
                                    self.last_done_text = sample.to_owned();
                                    self.done_entered_at = Some(Instant::now());
                                }
                            }
                        }
                    });

                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(4.0);

                    ui.small("최근 AI(MCP) 작업 기록");
                    if self.logs.is_empty() {
                        ui.weak("아직 실행된 AI 작업이 없습니다.");
                    } else {
                        for entry in self.logs.iter().take(5) {
                            ui.horizontal(|ui| {
                                let (r, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                                entry.category.paint_icon(ui.painter(), r.center());
                                let secs = entry.at.elapsed().as_secs();
                                let ago = if secs < 60 {
                                    format!("{secs}초 전")
                                } else {
                                    format!("{}분 전", secs / 60)
                                };
                                ui.small(format!("{} ({ago})", entry.summary));
                            });
                        }
                    }

                    ui.add_space(4.0);
                    ui.weak("숨긴 뒤에는 상단 바 '+' → 'AI 에이전트'로 다시 켤 수 있어요.");
                });
            self.popover_open = open;
        }

        actions
    }
}
