//! Tablet touch, stylus, Left-Top Offset Mouse Cursor, and Shortcut Pad handler (V8 · egui 0.37).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

const PALM_REJECTION_WINDOW: Duration = Duration::from_millis(450);
const LONG_PRESS_DURATION: Duration = Duration::from_millis(550);
const LONG_PRESS_SLOP_PT: f32 = 12.0;
const DOUBLE_TAP_WINDOW: Duration = Duration::from_millis(320);
const DOUBLE_TAP_SNAP_PT: f32 = 24.0;
const MOUSE_TAP_SLOP_PT: f32 = 7.0;
const MOUSE_TAP_MAX_DURATION: Duration = Duration::from_millis(260);
const MOUSE_DOUBLE_TAP_WINDOW: Duration = Duration::from_millis(340);
const MOUSE_OFFSET_X: f32 = -36.0;
const MOUSE_OFFSET_Y: f32 = -44.0;

#[derive(Debug, Default, Clone, Copy)]
pub struct TabletModifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub space_pan: bool,
}

impl TabletModifiers {
    pub fn to_egui_modifiers(self) -> egui::Modifiers {
        egui::Modifiers {
            alt: self.alt,
            ctrl: self.ctrl,
            shift: self.shift,
            mac_cmd: false,
            command: self.ctrl,
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ShortcutPadActions {
    pub undo: bool,
    pub redo: bool,
    pub paste: bool,
    pub toggle_ime: bool,
    pub toggle_mouse: bool,
}

#[derive(Debug, Clone, Copy)]
struct LongPressCandidate {
    touch_id: egui::TouchId,
    origin: egui::Pos2,
    started_at: Instant,
    triggered: bool,
}

#[derive(Debug, Clone, Copy)]
struct MouseTouchTrack {
    touch_id: egui::TouchId,
    down_finger_pos: egui::Pos2,
    down_at: Instant,
    moved_far: bool,
    is_dragging: bool,
}

pub struct TabletInputState {
    pub modifiers: TabletModifiers,
    pub show_shortcut_pad: bool,
    pub long_press_enabled: bool,
    pub mouse_cursor_mode: bool,
    pub multi_select_mode: bool,
    pub virtual_cursor_pos: egui::Pos2,
    pub stylus_active: bool,
    pub last_stylus_time: Option<Instant>,
    pub last_stylus_pos_pt: egui::Pos2,
    pub last_stylus_pressure: f32,
    pub last_stylus_tilt: (f32, f32),
    pub last_stylus_eraser: bool,
    pub safe_insets_px: [f32; 4],
    pub ime_height_px: f32,
    pub pixels_per_point: f32,
    pub top_bar_bottom_pt: f32,
    pub shortcut_pad_rect: Option<egui::Rect>,
    pub shortcut_pad_pos: Option<egui::Pos2>,
    pub canvas_rect: Option<egui::Rect>,
    pub screen_rect: egui::Rect,

    active_touches: BTreeMap<egui::TouchId, egui::Pos2>,
    was_multi_touch: bool,
    last_pinch_dist: Option<f32>,
    last_pinch_center: Option<egui::Pos2>,
    hand_pan_last_pos: Option<egui::Pos2>,
    long_press: Option<LongPressCandidate>,
    suppressed_touch_id: Option<egui::TouchId>,
    last_tap: Option<(Instant, egui::Pos2)>,
    last_sent_modifiers: egui::Modifiers,
    shortcut_pad_id: egui::Id,
    mouse_track: Option<MouseTouchTrack>,
    last_mouse_tap: Option<(Instant, egui::Pos2)>,
}

impl Default for TabletInputState {
    fn default() -> Self {
        Self {
            modifiers: TabletModifiers::default(),
            show_shortcut_pad: true,
            long_press_enabled: true,
            mouse_cursor_mode: false,
            multi_select_mode: false,
            virtual_cursor_pos: egui::pos2(300.0, 300.0),
            stylus_active: false,
            last_stylus_time: None,
            last_stylus_pos_pt: egui::Pos2::ZERO,
            last_stylus_pressure: 0.0,
            last_stylus_tilt: (0.0, 0.0),
            last_stylus_eraser: false,
            safe_insets_px: [0.0; 4],
            ime_height_px: 0.0,
            pixels_per_point: 2.0,
            top_bar_bottom_pt: 52.0,
            shortcut_pad_rect: None,
            shortcut_pad_pos: None,
            canvas_rect: None,
            screen_rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0)),
            active_touches: BTreeMap::new(),
            was_multi_touch: false,
            last_pinch_dist: None,
            last_pinch_center: None,
            hand_pan_last_pos: None,
            long_press: None,
            suppressed_touch_id: None,
            last_tap: None,
            last_sent_modifiers: egui::Modifiers::NONE,
            shortcut_pad_id: egui::Id::unique("figcraft_tablet_shortcut_pad"),
            mouse_track: None,
            last_mouse_tap: None,
        }
    }
}

impl TabletInputState {
    pub fn set_pixels_per_point(&mut self, ppp: f32) {
        if ppp > 0.1 {
            self.pixels_per_point = ppp;
        }
    }

    pub fn is_palm_rejection_active(&self) -> bool {
        if self.stylus_active {
            return true;
        }
        self.last_stylus_time
            .map(|t| t.elapsed() < PALM_REJECTION_WINDOW)
            .unwrap_or(false)
    }

    pub fn needs_repaint(&self) -> bool {
        self.long_press.as_ref().is_some_and(|lp| !lp.triggered)
    }

    pub fn on_stylus_event(
        &mut self,
        action: &str,
        x: f32,
        y: f32,
        pressure: f32,
        tilt_x: f32,
        tilt_y: f32,
        is_eraser: bool,
    ) {
        let ppp = self.pixels_per_point.max(1.0);
        match action {
            "down" | "move" | "hover" => {
                self.stylus_active = true;
                self.last_stylus_time = Some(Instant::now());
                self.last_stylus_pos_pt = egui::pos2(x / ppp, y / ppp);
                self.last_stylus_pressure = pressure.clamp(0.0, 1.0);
                self.last_stylus_tilt = (tilt_x, tilt_y);
                self.last_stylus_eraser = is_eraser;
                self.long_press = None;
            }
            "up" => {
                self.stylus_active = false;
                self.last_stylus_time = Some(Instant::now());
                self.last_stylus_pressure = 0.0;
            }
            _ => {}
        }
    }

    fn is_in_shell_chrome(&self, pos: egui::Pos2) -> bool {
        if pos.y <= self.top_bar_bottom_pt {
            return true;
        }
        if let Some(pad_rect) = self.shortcut_pad_rect {
            if pad_rect.contains(pos) {
                return true;
            }
        }
        false
    }

    fn is_inside_canvas(&self, pos: egui::Pos2) -> bool {
        if self.is_in_shell_chrome(pos) {
            return false;
        }
        if let Some(canvas) = self.canvas_rect {
            return canvas.contains(pos);
        }
        true
    }

    /// Maps finger position to Left-Top Offset Cursor position, easing smoothly near right/bottom edges.
    pub fn finger_to_cursor_pos(&self, finger: egui::Pos2) -> egui::Pos2 {
        let max_x = self.screen_rect.right().max(100.0);
        let max_y = self.screen_rect.bottom().max(100.0);
        let edge_zone = 96.0;
        let rx = ((max_x - finger.x) / edge_zone).clamp(0.0, 1.0);
        let ry = ((max_y - finger.y) / edge_zone).clamp(0.0, 1.0);
        let ox = MOUSE_OFFSET_X * rx;
        let oy = MOUSE_OFFSET_Y * ry;
        egui::pos2(
            (finger.x + ox).clamp(0.0, max_x - 1.0),
            (finger.y + oy).clamp(0.0, max_y - 1.0),
        )
    }

    pub fn process_raw_input(&mut self, raw_input: &mut egui::RawInput) {
        let mut active_modifiers = self.modifiers.to_egui_modifiers();
        if self.multi_select_mode {
            active_modifiers.shift = true;
        }
        let mut synthetic_events: Vec<egui::Event> = Vec::new();
        if active_modifiers != self.last_sent_modifiers {
            self.last_sent_modifiers = active_modifiers;
            synthetic_events.push(egui::Event::ModifiersChanged(active_modifiers));
        }

        let palm_active = self.is_palm_rejection_active();

        // Left-Top Offset Mouse Cursor Mode (only for finger touches outside top bar / Shortcut Pad)
        if self.mouse_cursor_mode && !palm_active {
            if self.process_mouse_cursor_mode(raw_input, active_modifiers, &mut synthetic_events) {
                raw_input.events.extend(synthetic_events);
                return;
            }
        } else {
            self.mouse_track = None;
        }

        let prev_touch_count = self.active_touches.len();
        let mut hand_pan_delta = egui::Vec2::ZERO;
        let mut suppress_this_frame = false;
        let mut snap_double_tap: Option<egui::Pos2> = None;

        for event in &raw_input.events {
            if let egui::Event::Touch {
                id, phase, pos, ..
            } = event
            {
                let in_canvas = self.is_inside_canvas(*pos);
                match phase {
                    egui::TouchPhase::Start => {
                        self.active_touches.insert(*id, *pos);
                        if self.active_touches.len() == 1 && !palm_active && in_canvas {
                            if self.modifiers.space_pan {
                                self.hand_pan_last_pos = Some(*pos);
                                self.long_press = None;
                            } else {
                                if let Some((prev_t, prev_pos)) = self.last_tap {
                                    if prev_t.elapsed() <= DOUBLE_TAP_WINDOW
                                        && prev_pos.distance(*pos) <= DOUBLE_TAP_SNAP_PT
                                    {
                                        snap_double_tap = Some(prev_pos);
                                    }
                                }
                                if self.long_press_enabled {
                                    self.long_press = Some(LongPressCandidate {
                                        touch_id: *id,
                                        origin: *pos,
                                        started_at: Instant::now(),
                                        triggered: false,
                                    });
                                } else {
                                    self.long_press = None;
                                }
                            }
                        } else {
                            self.long_press = None;
                            self.hand_pan_last_pos = None;
                        }
                    }
                    egui::TouchPhase::Move => {
                        self.active_touches.insert(*id, *pos);
                        if self.active_touches.len() == 1 && self.modifiers.space_pan && !palm_active {
                            if let Some(prev) = self.hand_pan_last_pos {
                                hand_pan_delta += *pos - prev;
                                self.hand_pan_last_pos = Some(*pos);
                            } else if in_canvas {
                                self.hand_pan_last_pos = Some(*pos);
                            }
                        }
                        if let Some(lp) = &self.long_press {
                            if lp.touch_id == *id && lp.origin.distance(*pos) > LONG_PRESS_SLOP_PT {
                                self.long_press = None;
                            }
                        }
                    }
                    egui::TouchPhase::End | egui::TouchPhase::Cancel => {
                        self.active_touches.remove(id);
                        self.hand_pan_last_pos = None;
                        if self.suppressed_touch_id == Some(*id) {
                            suppress_this_frame = true;
                            self.suppressed_touch_id = None;
                        } else if matches!(phase, egui::TouchPhase::End)
                            && self.active_touches.is_empty()
                            && !self.was_multi_touch
                            && !palm_active
                            && in_canvas
                        {
                            self.last_tap = Some((Instant::now(), *pos));
                        }
                        if self.long_press.is_some_and(|lp| lp.touch_id == *id) {
                            self.long_press = None;
                        }
                    }
                }
            }
        }

        if let Some(snap_pos) = snap_double_tap {
            for ev in &mut raw_input.events {
                if let egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    ..
                } = ev
                {
                    *pos = snap_pos;
                }
            }
        }

        if suppress_this_frame {
            raw_input.events.retain(|ev| {
                !matches!(
                    ev,
                    egui::Event::Touch { .. }
                        | egui::Event::PointerButton {
                            button: egui::PointerButton::Primary,
                            ..
                        }
                        | egui::Event::PointerMoved(_)
                )
            });
        }

        if self.modifiers.space_pan
            && self.active_touches.len() == 1
            && !palm_active
            && self.hand_pan_last_pos.is_some()
        {
            raw_input.events.retain(|ev| {
                !matches!(
                    ev,
                    egui::Event::Touch { .. }
                        | egui::Event::PointerButton {
                            button: egui::PointerButton::Primary,
                            ..
                        }
                        | egui::Event::PointerMoved(_)
                )
            });
            if hand_pan_delta.length_sq() > 0.01 {
                synthetic_events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: hand_pan_delta,
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Unknown,
                    modifiers: active_modifiers,
                });
            }
        }

        if self.active_touches.len() >= 2 && !palm_active {
            if !self.was_multi_touch && prev_touch_count == 1 {
                if let Some(&first_pos) = self.active_touches.values().next() {
                    synthetic_events.push(egui::Event::PointerButton {
                        pos: first_pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: active_modifiers,
                    });
                }
            }
            self.was_multi_touch = true;

            raw_input.events.retain(|ev| {
                !matches!(
                    ev,
                    egui::Event::Touch { .. }
                        | egui::Event::PointerButton {
                            button: egui::PointerButton::Primary,
                            ..
                        }
                        | egui::Event::PointerMoved(_)
                )
            });

            let mut iter = self.active_touches.values();
            if let (Some(&p1), Some(&p2)) = (iter.next(), iter.next()) {
                let dist = p1.distance(p2).max(1.0);
                let center = egui::pos2((p1.x + p2.x) * 0.5, (p1.y + p2.y) * 0.5);
                // Keep hover_pos anchored at pinch midpoint so FigCraft canvas accepts Zoom & Wheel
                synthetic_events.push(egui::Event::PointerMoved(center));

                if let (Some(prev_dist), Some(prev_center)) =
                    (self.last_pinch_dist, self.last_pinch_center)
                {
                    let zoom_ratio = (dist / prev_dist).clamp(0.5, 2.0);
                    if (zoom_ratio - 1.0).abs() > 0.002 {
                        synthetic_events.push(egui::Event::Zoom(zoom_ratio));
                    }
                    let pan_delta = center - prev_center;
                    if pan_delta.length_sq() > 0.05 {
                        synthetic_events.push(egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: pan_delta,
                            phase: egui::TouchPhase::Move,
                            source: egui::MouseWheelSource::Unknown,
                            modifiers: active_modifiers,
                        });
                    }
                }
                self.last_pinch_dist = Some(dist);
                self.last_pinch_center = Some(center);
            }
        } else {
            self.last_pinch_dist = None;
            self.last_pinch_center = None;
            if self.active_touches.is_empty() {
                self.was_multi_touch = false;
            }
        }

        if let Some(lp) = &mut self.long_press {
            if !lp.triggered && lp.started_at.elapsed() >= LONG_PRESS_DURATION {
                lp.triggered = true;
                self.suppressed_touch_id = Some(lp.touch_id);
                synthetic_events.push(egui::Event::PointerButton {
                    pos: lp.origin,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: active_modifiers,
                });
                synthetic_events.push(egui::Event::PointerMoved(lp.origin));
                synthetic_events.push(egui::Event::PointerButton {
                    pos: lp.origin,
                    button: egui::PointerButton::Secondary,
                    pressed: true,
                    modifiers: active_modifiers,
                });
                synthetic_events.push(egui::Event::PointerButton {
                    pos: lp.origin,
                    button: egui::PointerButton::Secondary,
                    pressed: false,
                    modifiers: active_modifiers,
                });
            }
        }

        if active_modifiers.any() {
            for ev in &mut raw_input.events {
                if let egui::Event::PointerButton { pos, modifiers, .. } = ev {
                    if !self.is_in_shell_chrome(*pos) {
                        *modifiers = *modifiers | active_modifiers;
                    }
                }
            }
        }
        raw_input.events.extend(synthetic_events);
    }

    /// Handles Left-Top Offset Mouse Cursor Mode:
    /// - Single-finger drag: Hover move (cursor follows left-top of finger, no click)
    /// - Single-finger tap: Left-click at cursor position
    /// - Double-tap & hold drag: Left-button drag at cursor position
    /// - Two-finger pinch/pan: Zoom & pan canvas
    fn process_mouse_cursor_mode(
        &mut self,
        raw_input: &mut egui::RawInput,
        active_modifiers: egui::Modifiers,
        synthetic_events: &mut Vec<egui::Event>,
    ) -> bool {
        let mut saw_touch = false;
        let mut direct_chrome_touch = false;

        for event in &raw_input.events {
            if let egui::Event::Touch { id, phase, pos, .. } = event {
                saw_touch = true;
                if matches!(phase, egui::TouchPhase::Start)
                    && self.active_touches.is_empty()
                    && self.is_in_shell_chrome(*pos)
                {
                    direct_chrome_touch = true;
                }
                match phase {
                    egui::TouchPhase::Start => {
                        self.active_touches.insert(*id, *pos);
                        if !direct_chrome_touch && self.active_touches.len() == 1 {
                            let cursor = self.finger_to_cursor_pos(*pos);
                            self.virtual_cursor_pos = cursor;
                            let is_double_tap = self.last_mouse_tap.is_some_and(|(t, prev_f)| {
                                t.elapsed() <= MOUSE_DOUBLE_TAP_WINDOW
                                    && prev_f.distance(*pos) <= 32.0
                            });
                            self.mouse_track = Some(MouseTouchTrack {
                                touch_id: *id,
                                down_finger_pos: *pos,
                                down_at: Instant::now(),
                                moved_far: false,
                                is_dragging: is_double_tap,
                            });
                            synthetic_events.push(egui::Event::PointerMoved(cursor));
                            if is_double_tap {
                                self.last_mouse_tap = None;
                                synthetic_events.push(egui::Event::PointerButton {
                                    pos: cursor,
                                    button: egui::PointerButton::Primary,
                                    pressed: true,
                                    modifiers: active_modifiers,
                                });
                            }
                        } else if self.active_touches.len() >= 2 {
                            // Cancel any drag before pinch/pan
                            if let Some(tr) = self.mouse_track.take() {
                                if tr.is_dragging {
                                    synthetic_events.push(egui::Event::PointerButton {
                                        pos: self.virtual_cursor_pos,
                                        button: egui::PointerButton::Primary,
                                        pressed: false,
                                        modifiers: active_modifiers,
                                    });
                                }
                            }
                        }
                    }
                    egui::TouchPhase::Move => {
                        self.active_touches.insert(*id, *pos);
                        if self.active_touches.len() == 1 {
                            if let Some(tr) = &mut self.mouse_track {
                                if tr.touch_id == *id {
                                    if tr.down_finger_pos.distance(*pos) > MOUSE_TAP_SLOP_PT {
                                        tr.moved_far = true;
                                    }
                                    let cursor = self.finger_to_cursor_pos(*pos);
                                    self.virtual_cursor_pos = cursor;
                                    synthetic_events.push(egui::Event::PointerMoved(cursor));
                                }
                            }
                        }
                    }
                    egui::TouchPhase::End | egui::TouchPhase::Cancel => {
                        self.active_touches.remove(id);
                        if let Some(tr) = self.mouse_track {
                            if tr.touch_id == *id {
                                self.mouse_track = None;
                                let cursor = self.virtual_cursor_pos;
                                if tr.is_dragging {
                                    synthetic_events.push(egui::Event::PointerButton {
                                        pos: cursor,
                                        button: egui::PointerButton::Primary,
                                        pressed: false,
                                        modifiers: active_modifiers,
                                    });
                                } else if matches!(phase, egui::TouchPhase::End)
                                    && !tr.moved_far
                                    && tr.down_at.elapsed() <= MOUSE_TAP_MAX_DURATION
                                {
                                    self.last_mouse_tap = Some((Instant::now(), tr.down_finger_pos));
                                    synthetic_events.push(egui::Event::PointerMoved(cursor));
                                    synthetic_events.push(egui::Event::PointerButton {
                                        pos: cursor,
                                        button: egui::PointerButton::Primary,
                                        pressed: true,
                                        modifiers: active_modifiers,
                                    });
                                    synthetic_events.push(egui::Event::PointerButton {
                                        pos: cursor,
                                        button: egui::PointerButton::Primary,
                                        pressed: false,
                                        modifiers: active_modifiers,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }

        if direct_chrome_touch {
            self.mouse_track = None;
            return false;
        }

        if !saw_touch && self.mouse_track.is_none() && self.active_touches.is_empty() {
            return false;
        }

        // Handle 2-finger pinch/pan cleanly in mouse mode too
        if self.active_touches.len() >= 2 {
            self.was_multi_touch = true;
            let mut iter = self.active_touches.values();
            if let (Some(&p1), Some(&p2)) = (iter.next(), iter.next()) {
                let dist = p1.distance(p2).max(1.0);
                let center = egui::pos2((p1.x + p2.x) * 0.5, (p1.y + p2.y) * 0.5);
                synthetic_events.push(egui::Event::PointerMoved(center));
                if let (Some(prev_dist), Some(prev_center)) =
                    (self.last_pinch_dist, self.last_pinch_center)
                {
                    let zoom_ratio = (dist / prev_dist).clamp(0.5, 2.0);
                    if (zoom_ratio - 1.0).abs() > 0.002 {
                        synthetic_events.push(egui::Event::Zoom(zoom_ratio));
                    }
                    let pan_delta = center - prev_center;
                    if pan_delta.length_sq() > 0.05 {
                        synthetic_events.push(egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: pan_delta,
                            phase: egui::TouchPhase::Move,
                            source: egui::MouseWheelSource::Unknown,
                            modifiers: active_modifiers,
                        });
                    }
                }
                self.last_pinch_dist = Some(dist);
                self.last_pinch_center = Some(center);
            }
        } else {
            self.last_pinch_dist = None;
            self.last_pinch_center = None;
            if self.active_touches.is_empty() {
                self.was_multi_touch = false;
            }
        }

        // Strip raw finger touch/pointer events so only our offset cursor events reach egui
        raw_input.events.retain(|ev| {
            !matches!(
                ev,
                egui::Event::Touch { .. }
                    | egui::Event::PointerButton {
                        button: egui::PointerButton::Primary,
                        ..
                    }
                    | egui::Event::PointerMoved(_)
            )
        });
        true
    }

    /// Draws the glowing Left-Top Offset Mouse Cursor overlay on the foreground layer.
    pub fn paint_virtual_cursor(&self, ctx: &egui::Context) {
        if !self.mouse_cursor_mode || self.is_palm_rejection_active() {
            return;
        }
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            egui::Id::unique("figcraft_virtual_mouse_cursor"),
        ));
        let p = self.virtual_cursor_pos;
        let dragging = self.mouse_track.is_some_and(|t| t.is_dragging);
        let accent = if dragging {
            egui::Color32::from_rgb(251, 146, 60) // orange while dragging
        } else {
            egui::Color32::from_rgb(56, 189, 248) // cyan while aiming
        };

        painter.circle_stroke(p, if dragging { 11.0 } else { 8.0 }, egui::Stroke::new(1.5, accent));
        let arrow = vec![
            p,
            p + egui::vec2(0.0, 17.0),
            p + egui::vec2(4.5, 13.0),
            p + egui::vec2(8.0, 19.5),
            p + egui::vec2(10.5, 18.0),
            p + egui::vec2(7.0, 11.5),
            p + egui::vec2(12.5, 11.5),
        ];
        painter.add(egui::Shape::convex_polygon(
            arrow,
            egui::Color32::WHITE,
            egui::Stroke::new(1.5, egui::Color32::from_rgb(15, 23, 42)),
        ));
    }

    pub fn show_shortcut_pad_ui(&mut self, ctx: &egui::Context) -> ShortcutPadActions {
        let mut actions = ShortcutPadActions::default();
        if !self.show_shortcut_pad {
            self.shortcut_pad_rect = None;
            return actions;
        }

        let ppp = self.pixels_per_point.max(1.0);
        let top_min = self.top_bar_bottom_pt + 6.0;
        let right_inset = self.safe_insets_px[1] / ppp;
        let bottom_margin = (self.safe_insets_px[2].max(self.ime_height_px) / ppp) + 12.0;
        let left_margin = (self.safe_insets_px[3] / ppp) + 12.0;

        let vp = ctx.input(|i| i.viewport_rect());
        let screen_w = vp.width().max(320.0);
        let screen_h = vp.height().max(320.0);
        let max_pad_w = (screen_w - left_margin - right_inset - 12.0).max(260.0);
        let pad_h = 52.0;

        let default_pos = egui::pos2(left_margin, (screen_h - bottom_margin - pad_h).max(top_min));
        let mut pad_pos = self.shortcut_pad_pos.unwrap_or(default_pos);
        let est_w = self
            .shortcut_pad_rect
            .map(|r| r.width())
            .unwrap_or(560.0)
            .min(max_pad_w);
        pad_pos.x = pad_pos.x.clamp(left_margin, (screen_w - right_inset - est_w).max(left_margin));
        pad_pos.y = pad_pos.y.clamp(top_min, (screen_h - bottom_margin - pad_h).max(top_min));

        let mut drag_delta = egui::Vec2::ZERO;
        let btn_h = 36.0;
        let handle_size = egui::vec2(26.0, btn_h);

        let area_resp = egui::Area::new(self.shortcut_pad_id)
            .fixed_pos(pad_pos)
            .order(egui::Order::Foreground)
            .interactable(true)
            .show(ctx, |ui| {
                ui.set_max_width(max_pad_w);
                egui::Frame::window(ui.style())
                    .corner_radius(egui::CornerRadius::same(14))
                    .inner_margin(egui::Margin::symmetric(6, 6))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 5.0;

                            // Left drag handle
                            let left_handle = ui.add_sized(
                                handle_size,
                                egui::Button::new("⋮⋮").sense(egui::Sense::drag()),
                            );
                            if left_handle.dragged() {
                                drag_delta += left_handle.drag_delta();
                            }

                            egui::ScrollArea::horizontal()
                                .id_salt("shortcut_pad_scroll")
                                .max_width((max_pad_w - 76.0).max(160.0))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 5.0;
                                        if ui
                                            .add(egui::Button::new("Shift").selected(self.modifiers.shift).min_size(egui::vec2(44.0, btn_h)))
                                            .clicked()
                                        {
                                            self.modifiers.shift = !self.modifiers.shift;
                                        }
                                        if ui
                                            .add(egui::Button::new("Ctrl").selected(self.modifiers.ctrl).min_size(egui::vec2(42.0, btn_h)))
                                            .clicked()
                                        {
                                            self.modifiers.ctrl = !self.modifiers.ctrl;
                                        }
                                        if ui
                                            .add(egui::Button::new("Alt").selected(self.modifiers.alt).min_size(egui::vec2(38.0, btn_h)))
                                            .clicked()
                                        {
                                            self.modifiers.alt = !self.modifiers.alt;
                                        }
                                        if ui
                                            .add(egui::Button::new("손바닥").selected(self.modifiers.space_pan).min_size(egui::vec2(50.0, btn_h)))
                                            .clicked()
                                        {
                                            self.modifiers.space_pan = !self.modifiers.space_pan;
                                        }
                                        if ui
                                            .add(egui::Button::new("그리기").selected(!self.long_press_enabled).min_size(egui::vec2(50.0, btn_h)))
                                            .clicked()
                                        {
                                            self.long_press_enabled = !self.long_press_enabled;
                                            if !self.long_press_enabled {
                                                self.long_press = None;
                                            }
                                        }
                                        if ui
                                            .add(egui::Button::new("마우스").selected(self.mouse_cursor_mode).min_size(egui::vec2(50.0, btn_h)))
                                            .clicked()
                                        {
                                            actions.toggle_mouse = true;
                                        }
                                        ui.separator();
                                        if ui.add(egui::Button::new("붙여넣기").min_size(egui::vec2(58.0, btn_h))).clicked() {
                                            actions.paste = true;
                                        }
                                        if ui.add(egui::Button::new("실행취소").min_size(egui::vec2(58.0, btn_h))).clicked() {
                                            actions.undo = true;
                                        }
                                        if ui.add(egui::Button::new("다시실행").min_size(egui::vec2(58.0, btn_h))).clicked() {
                                            actions.redo = true;
                                        }
                                        if ui.add(egui::Button::new("키보드").min_size(egui::vec2(50.0, btn_h))).clicked() {
                                            actions.toggle_ime = true;
                                        }
                                    });
                                });

                            // Right drag handle
                            let right_handle = ui.add_sized(
                                handle_size,
                                egui::Button::new("⋮⋮").sense(egui::Sense::drag()),
                            );
                            if right_handle.dragged() {
                                drag_delta += right_handle.drag_delta();
                            }
                        });
                    });
            });

        if drag_delta.length_sq() > 0.01 {
            pad_pos += drag_delta;
            pad_pos.x = pad_pos.x.clamp(left_margin, (screen_w - right_inset - est_w).max(left_margin));
            pad_pos.y = pad_pos.y.clamp(top_min, (screen_h - bottom_margin - pad_h).max(top_min));
            self.shortcut_pad_pos = Some(pad_pos);
            ctx.request_repaint();
        }
        self.shortcut_pad_rect = Some(area_resp.response.rect);
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_mouse_cursor_maps_left_top_and_clamps() {
        let state = TabletInputState::default();
        let c = state.finger_to_cursor_pos(egui::pos2(400.0, 300.0));
        assert!((c.x - 364.0).abs() < 0.5);
        assert!((c.y - 256.0).abs() < 0.5);

        let edge = state.finger_to_cursor_pos(egui::pos2(1280.0, 800.0));
        assert!(edge.x >= 1278.0 && edge.y >= 798.0);
    }

    #[test]
    fn outside_canvas_touch_never_starts_long_press() {
        let mut state = TabletInputState::default();
        state.canvas_rect = Some(egui::Rect::from_min_max(
            egui::pos2(240.0, 80.0),
            egui::pos2(1200.0, 900.0),
        ));
        let mut f1 = egui::RawInput::default();
        f1.events.push(egui::Event::Touch {
            device_id: egui::TouchDeviceId(1),
            id: egui::TouchId(1),
            phase: egui::TouchPhase::Start,
            pos: egui::pos2(50.0, 200.0),
            force: None,
        });
        state.process_raw_input(&mut f1);
        assert!(state.long_press.is_none());
    }
}
