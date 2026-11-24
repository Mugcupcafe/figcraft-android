//! Floating draggable circular AI Agent mascot bubble + dedicated AI canvas cursor &
//! created/modified element highlights for FigCraft Mobile (V26).

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
const WORKING_HOLD_SECS: f32 = 2.0;
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
    pub ok: bool,
    pub at: Instant,
}

#[derive(Debug, Default)]
pub struct AgentBubbleActions {
    pub undo: bool,
    pub prefs_dirty: bool,
}

struct AgentTextures {
    idle: egui::TextureHandle,
    working: egui::TextureHandle,
    done: egui::TextureHandle,
    paused: egui::TextureHandle,
}

struct DemoStep {
    rel_rect: [f32; 4], // normalized [x, y, w, h] inside canvas_rect
    label: &'static str,
    action_label: &'static str,
    kind: HighlightKind,
}

const DEMO_STEPS: &[DemoStep] = &[
    DemoStep {
        rel_rect: [0.18, 0.22, 0.28, 0.22],
        label: "Hero Card",
        action_label: "프레임 생성 중",
        kind: HighlightKind::Created,
    },
    DemoStep {
        rel_rect: [0.22, 0.27, 0.20, 0.06],
        label: "제목 텍스트",
        action_label: "텍스트 수정 중",
        kind: HighlightKind::Modified,
    },
    DemoStep {
        rel_rect: [0.54, 0.46, 0.22, 0.14],
        label: "CTA 버튼",
        action_label: "스타일 꾸미는 중",
        kind: HighlightKind::Created,
    },
];

pub struct AgentBubbleState {
    textures: Option<AgentTextures>,
    pub paused: bool,
    preview_override: Option<(AgentVisualState, Instant)>,
    last_work_at: Option<Instant>,
    done_entered_at: Option<Instant>,
    last_visual_state: AgentVisualState,
    burst_count: u32,
    current_task_text: String,
    last_done_text: String,
    logs: VecDeque<AgentLogEntry>,
    pub popover_open: bool,
    popover_open_seq: u64,
    dragging: bool,
    drag_total_dist: f32,
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
            preview_override: None,
            last_work_at: None,
            done_entered_at: None,
            last_visual_state: AgentVisualState::Idle,
            burst_count: 0,
            current_task_text: String::new(),
            last_done_text: String::new(),
            logs: VecDeque::with_capacity(MAX_LOG_ENTRIES),
            popover_open: false,
            popover_open_seq: 0,
            dragging: false,
            drag_total_dist: 0.0,
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
    pub fn friendly_command_label(cmd: &str) -> String {
        let c = cmd.trim();
        if c.starts_with("node.create") || c.starts_with("shape.") || c.starts_with("frame.") || c == "create_node" {
            "도형·프레임 그리는 중".into()
        } else if c.starts_with("text.") || c == "type_text" {
            "텍스트 편집 중".into()
        } else if c.starts_with("node.set")
            || c == "set_properties"
            || c.starts_with("style.")
            || c.starts_with("fill.")
            || c.starts_with("stroke.")
            || c.starts_with("effect.")
        {
            "스타일 꾸미는 중".into()
        } else if c.starts_with("node.move")
            || c.starts_with("node.resize")
            || c.starts_with("layout.")
            || c.starts_with("align.")
            || c.starts_with("distribute.")
            || c.starts_with("transform.")
        {
            "레이아웃 정리 중".into()
        } else if c.starts_with("node.delete") || c.starts_with("edit.delete") {
            "요소 정리 중".into()
        } else if c.starts_with("image.") {
            "이미지 배치 중".into()
        } else if c.starts_with("doc.") || c.starts_with("page.") || c.starts_with("node.get") || c.starts_with("node.list") {
            "캔버스 구조 확인 중".into()
        } else if c.starts_with("edit.undo") || c.starts_with("edit.redo") {
            "작업 되돌리는 중".into()
        } else if c == "figcraft.status" {
            "연결 상태 확인 중".into()
        } else if c.is_empty() {
            "AI 작업 중".into()
        } else {
            format!("{c} 실행 중")
        }
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
        let summary = Self::friendly_command_label(raw_cmd);
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
            ok,
            at: now,
        });

        // Update AI canvas cursor & highlight target when a visual edit happened
        if ok && raw_cmd != "figcraft.status" {
            let bounds = output
                .and_then(Self::extract_bounds_from_value)
                .or_else(|| params.and_then(Self::extract_bounds_from_value));
            let kind = Self::classify_kind(raw_cmd);

            if let Some(canvas) = canvas_rect {
                let (screen_rect, elem_name) = if let Some((px, py, pw, ph, name)) = bounds {
                    // Map page coordinates into canvas viewport cleanly, clamped inside visible canvas
                    let origin = canvas.left_top() + Vec2::new(60.0, 60.0);
                    let raw_min = origin + Vec2::new(px, py);
                    let clamped_w = pw.clamp(28.0, (canvas.width() * 0.72).max(40.0));
                    let clamped_h = ph.clamp(24.0, (canvas.height() * 0.68).max(32.0));
                    let min_x = raw_min
                        .x
                        .clamp(canvas.left() + 18.0, (canvas.right() - clamped_w - 18.0).max(canvas.left() + 18.0));
                    let min_y = raw_min
                        .y
                        .clamp(canvas.top() + 24.0, (canvas.bottom() - clamped_h - 24.0).max(canvas.top() + 24.0));
                    (
                        Rect::from_min_size(Pos2::new(min_x, min_y), Vec2::new(clamped_w, clamped_h)),
                        name,
                    )
                } else {
                    // No explicit coordinates in command: place a gentle focused marker near canvas center
                    let hash = raw_cmd.bytes().fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
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
                    Pos2::new(screen_rect.left() + screen_rect.width() * 0.25, screen_rect.top() + screen_rect.height() * 0.35),
                    format!("AI · {summary}"),
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
            // Start smoothly from the AI bubble position if known
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
            self.push_highlight(
                r,
                format!("{} · {}", step.kind.tag_ko(), step.label),
                step.kind,
                now,
            );
            self.set_cursor_target(
                Pos2::new(r.left() + r.width() * 0.22, r.top() + r.height() * 0.35),
                format!("AI · {}", step.action_label),
                now,
            );
            self.last_work_at = Some(now);
            self.current_task_text = step.action_label.to_owned();
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
        let cursor_alive = self
            .cursor_active_until
            .map(|u| now < u)
            .unwrap_or(false);
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
        now: Instant,
        dt: f32,
    ) {
        if !prefs.ai_cursor_enabled {
            return;
        }

        // Expire old highlights
        self.highlights
            .retain(|h| now.duration_since(h.created_at).as_secs_f32() < HIGHLIGHT_TTL_SECS);

        // Smoothly interpolate AI cursor toward target
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

                // 1. Paint created / modified element highlights
                for hl in &self.highlights {
                    let age = now.duration_since(hl.created_at).as_secs_f32();
                    let fade_in = (age / 0.18).clamp(0.0, 1.0);
                    let fade_out = ((HIGHLIGHT_TTL_SECS - age) / 0.7).clamp(0.0, 1.0);
                    let alpha = fade_in * fade_out;
                    let base = hl.kind.color();
                    let pulse = (t * 6.0).sin() * 0.5 + 0.5;

                    // Soft tinted fill + outer glow + crisp border
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

                    // Top-left compact badge pill (e.g., "생성 · Hero Card")
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

                // 2. Paint dedicated Figma-style AI Agent cursor + floating action label
                if cursor_remaining > 0.0 {
                    if let Some(tip) = self.cursor_pos {
                        let fade = (cursor_remaining / 0.6).clamp(0.0, 1.0);
                        let mint = Color32::from_rgba_unmultiplied(84, 238, 192, (250.0 * fade) as u8);
                        let outline = Color32::from_rgba_unmultiplied(18, 28, 32, (240.0 * fade) as u8);
                        let shadow = Color32::from_rgba_unmultiplied(0, 0, 0, (90.0 * fade) as u8);

                        // Crisp concave-safe pointer arrow (2 convex triangles + stem quad + outline)
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

                        // Attached rounded action label pill next to the cursor
                        if !self.cursor_label.is_empty() {
                            let galley = painter.layout_no_wrap(
                                self.cursor_label.clone(),
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

        self.tick_demo(eff_canvas, now);
        self.paint_canvas_cursor_and_highlights(ctx, prefs, eff_canvas, now, dt);

        if !prefs.ai_bubble_visible {
            self.popover_open = false;
            return actions;
        }

        self.ensure_textures(ctx);
        let state = self.current_state();
        let size = prefs.ai_bubble_size.clamp(36.0, 96.0);

        let min_x = screen.left() + 8.0;
        let max_x = (screen.right() - size - 8.0).max(min_x);
        let min_y = (top_bar_bottom + 10.0).max(screen.top() + 56.0);
        let max_y = (screen.bottom() - size - 18.0).max(min_y);

        let initial_pos = if let Some(p) = self.bubble_pos {
            p
        } else if prefs.ai_bubble_pos_x >= 0.0 && prefs.ai_bubble_pos_y >= 0.0 {
            Pos2::new(prefs.ai_bubble_pos_x, prefs.ai_bubble_pos_y)
        } else {
            Pos2::new(max_x - 10.0, (max_y - 68.0).max(min_y))
        };
        let mut pos = Pos2::new(
            initial_pos.x.clamp(min_x, max_x),
            initial_pos.y.clamp(min_y, max_y),
        );
        self.bubble_pos = Some(pos);

        let t = ctx.input(|i| i.time) as f32;

        // Character micro-animations (bobbing/breathing while Working, pop bounce on Done)
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

        // Floating draggable circular icon area
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

                if resp.drag_started() {
                    self.dragging = true;
                    self.drag_total_dist = 0.0;
                }
                if resp.dragged() {
                    let delta = resp.drag_delta();
                    self.drag_total_dist += delta.length();
                    pos.x = (pos.x + delta.x).clamp(min_x, max_x);
                    pos.y = (pos.y + delta.y).clamp(min_y, max_y);
                    self.bubble_pos = Some(pos);
                }
                if resp.drag_stopped() {
                    self.dragging = false;
                    if self.drag_total_dist >= 6.0 {
                        prefs.ai_bubble_pos_x = pos.x;
                        prefs.ai_bubble_pos_y = pos.y;
                        actions.prefs_dirty = true;
                    }
                }
                if resp.clicked() && self.drag_total_dist < 6.0 {
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

                    // Rotating 115-degree bright arc
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

                // Draw Mint Mochi Cat-Spirit sprite
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

                // Done: celebratory mint & gold confetti burst for 1.2s
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

        // Live speech pill next to the bubble (with animated dots while Working)
        let speech_text: Option<String> = if !prefs.ai_bubble_show_speech || self.popover_open {
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
                    Some(format!("{base}{dots}"))
                }
                AgentVisualState::Done => Some(if self.last_done_text.is_empty() {
                    "작업 완료!".into()
                } else {
                    self.last_done_text.clone()
                }),
                AgentVisualState::Paused => Some("일시정지됨".into()),
                AgentVisualState::Idle => None,
            }
        };

        if let Some(text) = speech_text {
            let on_right_half = (pos.x + size * 0.5) > screen.center().x;
            let pill_h = 28.0;
            let est_w = (text.chars().count() as f32 * 11.5 + 28.0).clamp(96.0, 250.0);
            let pill_x = if on_right_half {
                (pos.x - est_w - 8.0).max(screen.left() + 8.0)
            } else {
                (pos.x + size + 8.0).min(screen.right() - est_w - 8.0)
            };
            let pill_y = pos.y + (size - pill_h) * 0.5;

            egui::Area::new(egui::Id::unique("figcraft_ai_agent_speech"))
                .order(egui::Order::Foreground)
                .fixed_pos(Pos2::new(pill_x, pill_y))
                .interactable(false)
                .show(ctx, |ui| {
                    let frame = egui::Frame::NONE
                        .fill(Color32::from_rgba_unmultiplied(22, 26, 33, 235))
                        .stroke(Stroke::new(1.2, state.accent_color()))
                        .corner_radius(CornerRadius::same(14))
                        .inner_margin(egui::Margin::symmetric(11, 5));
                    frame.show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            let (dot_rect, _) =
                                ui.allocate_exact_size(Vec2::splat(7.0), Sense::hover());
                            ui.painter()
                                .circle_filled(dot_rect.center(), 3.5, state.accent_color());
                            ui.label(
                                egui::RichText::new(text)
                                    .font(FontId::proportional(12.5))
                                    .color(Color32::from_rgb(240, 246, 250)),
                            );
                        });
                    });
                });
        }

        // Tap popover with controls, size customization, cursor toggle, live demo, and recent activity
        if self.popover_open {
            let win_w = 284.0;
            let win_h = 395.0;
            let on_right_half = (pos.x + size * 0.5) > screen.center().x;
            let raw_x = if on_right_half {
                pos.x - win_w - 10.0
            } else {
                pos.x + size + 10.0
            };
            let raw_y = pos.y - 20.0;
            let pop_x = raw_x.clamp(screen.left() + 8.0, (screen.right() - win_w - 8.0).max(8.0));
            let pop_y = raw_y.clamp(min_y, (screen.bottom() - win_h - 12.0).max(min_y));

            let mut open = self.popover_open;
            egui::Window::new("AI 에이전트")
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
                        ui.strong(state.label_ko());
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
                                    .min_size(Vec2::new(84.0, 32.0)),
                            )
                            .clicked()
                        {
                            self.paused = !self.paused;
                            self.preview_override = None;
                        }
                        if ui
                            .add(egui::Button::new("실행취소").min_size(Vec2::new(84.0, 32.0)))
                            .clicked()
                        {
                            actions.undo = true;
                        }
                        if ui
                            .add(egui::Button::new("숨기기").min_size(Vec2::new(74.0, 32.0)))
                            .clicked()
                        {
                            prefs.ai_bubble_visible = false;
                            actions.prefs_dirty = true;
                        }
                    });

                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(4.0);

                    ui.strong(format!("아이콘 크기 ({:.0}pt)", prefs.ai_bubble_size));
                    ui.horizontal(|ui| {
                        for (lbl, val) in [("작게", 44.0_f32), ("보통", 56.0), ("크게", 72.0), ("특대", 88.0)] {
                            let sel = (prefs.ai_bubble_size - val).abs() < 2.0;
                            if ui
                                .add(egui::Button::new(lbl).selected(sel).min_size(Vec2::new(58.0, 26.0)))
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
                        .checkbox(&mut prefs.ai_bubble_show_speech, "실시간 작업 말풍선 표시")
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
                                .add(egui::Button::new("커서 데모 체험").min_size(Vec2::new(98.0, 24.0)))
                                .clicked()
                            {
                                self.start_cursor_demo();
                            }
                        });
                    });
                    ui.horizontal(|ui| {
                        for (lbl, st, sample) in [
                            ("평상시", AgentVisualState::Idle, ""),
                            ("작업중", AgentVisualState::Working, "도형·프레임 그리는 중"),
                            ("완료", AgentVisualState::Done, "완료 · 3개 작업"),
                            ("정지", AgentVisualState::Paused, ""),
                        ] {
                            if ui
                                .add(egui::Button::new(lbl).min_size(Vec2::new(58.0, 26.0)))
                                .clicked()
                            {
                                self.preview_override =
                                    Some((st, Instant::now() + Duration::from_secs(4)));
                                if st == AgentVisualState::Working {
                                    self.current_task_text = sample.to_owned();
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
                                let col = if entry.ok {
                                    Color32::from_rgb(72, 224, 134)
                                } else {
                                    Color32::from_rgb(240, 100, 100)
                                };
                                let (r, _) = ui.allocate_exact_size(Vec2::splat(7.0), Sense::hover());
                                ui.painter().circle_filled(r.center(), 3.2, col);
                                let secs = entry.at.elapsed().as_secs();
                                let ago = if secs < 60 {
                                    format!("{secs}초 전")
                                } else {
                                    format!("{}분 전", secs / 60)
                                };
                                ui.small(format!("{} ({ago})", entry.raw_cmd));
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
