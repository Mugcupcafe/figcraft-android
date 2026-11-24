//! Tablet touch, stylus, Left-Top Offset Mouse Cursor, and Shortcut Pad handler (V8 · egui 0.37).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

const PALM_REJECTION_WINDOW: Duration = Duration::from_millis(450);
const LONG_PRESS_DURATION: Duration = Duration::from_millis(550);
const LONG_PRESS_SLOP_PT: f32 = 12.0;
const DOUBLE_TAP_WINDOW: Duration = Duration::from_millis(420);
const DOUBLE_TAP_SNAP_PT: f32 = 36.0;
const MOUSE_TAP_SLOP_PT: f32 = 7.0;
const MOUSE_TAP_MAX_DURATION: Duration = Duration::from_millis(260);
const MOUSE_DOUBLE_TAP_WINDOW: Duration = Duration::from_millis(340);
const MOUSE_OFFSET_X: f32 = -36.0;
const MOUSE_OFFSET_Y: f32 = -44.0;
const PINCH_ZOOM_DEADZONE_PT: f32 = 10.0;
const PINCH_ZOOM_STEP_THRESHOLD: f32 = 0.015;

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
    last_finger_pos: egui::Pos2,
    down_at: Instant,
    moved_far: bool,
    is_dragging: bool,
    right_clicked: bool,
}

pub struct TabletInputState {
    pub modifiers: TabletModifiers,
    pub show_shortcut_pad: bool,
    pub long_press_enabled: bool,
    pub mouse_cursor_mode: bool,
    pub multi_select_mode: bool,
    pub virtual_cursor_pos: egui::Pos2,
    pub mouse_cursor_scale: f32,
    pub mouse_grip_distance: f32,
    pub mouse_grip_offset_x: f32,
    pub mouse_grip_offset_y: f32,
    pub mouse_grip_opacity: f32,
    pub mouse_border_width: f32,
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
    pinch_start_dist: Option<f32>,
    pinch_zoom_unlocked: bool,
    last_pinch_dist: Option<f32>,
    last_pinch_center: Option<egui::Pos2>,
    hand_pan_last_pos: Option<egui::Pos2>,
    long_press: Option<LongPressCandidate>,
    suppressed_touch_id: Option<egui::TouchId>,
    last_tap: Option<(Instant, egui::Pos2)>,
    double_tap_snap: Option<(egui::TouchId, egui::Pos2)>,
    last_sent_modifiers: egui::Modifiers,
    shortcut_pad_id: egui::Id,
    mouse_track: Option<MouseTouchTrack>,
    last_mouse_tap: Option<(Instant, egui::Pos2)>,
    chrome_touch_id: Option<egui::TouchId>,
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
            mouse_cursor_scale: 1.0,
            mouse_grip_distance: 52.0,
            mouse_grip_offset_x: 56.0,
            mouse_grip_offset_y: 56.0,
            mouse_grip_opacity: 0.45,
            mouse_border_width: 2.0,
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
            pinch_start_dist: None,
            pinch_zoom_unlocked: false,
            last_pinch_dist: None,
            last_pinch_center: None,
            hand_pan_last_pos: None,
            long_press: None,
            suppressed_touch_id: None,
            last_tap: None,
            double_tap_snap: None,
            last_sent_modifiers: egui::Modifiers::NONE,
            shortcut_pad_id: egui::Id::unique("figcraft_tablet_shortcut_pad"),
            mouse_track: None,
            last_mouse_tap: None,
            chrome_touch_id: None,
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
            || self.mouse_track.is_some_and(|tr| !tr.moved_far && !tr.is_dragging && !tr.right_clicked)
    }

    pub fn center_virtual_cursor(&mut self) {
        self.virtual_cursor_pos = self.screen_rect.center();
    }

    pub fn mouse_grip_radius(&self) -> f32 {
        (22.0 * self.mouse_cursor_scale.clamp(0.7, 1.6)).clamp(18.0, 36.0)
    }

    pub fn mouse_grip_center(&self) -> egui::Pos2 {
        let ox = self.mouse_grip_offset_x.clamp(-200.0, 200.0);
        let oy = self.mouse_grip_offset_y.clamp(-200.0, 200.0);
        let r = self.mouse_grip_radius();
        let min_x = self.screen_rect.left() + r;
        let max_x = (self.screen_rect.right() - r).max(min_x);
        let min_y = self.screen_rect.top() + r;
        let max_y = (self.screen_rect.bottom() - r).max(min_y);
        egui::pos2(
            (self.virtual_cursor_pos.x + ox).clamp(min_x, max_x),
            (self.virtual_cursor_pos.y + oy).clamp(min_y, max_y),
        )
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

    /// Pure center canvas region excluding top bar, doc tab bar, left/right side panels, and bottom tool dock.
    fn is_inside_center_canvas(&self, pos: egui::Pos2) -> bool {
        if !self.is_inside_canvas(pos) {
            return false;
        }
        let sr = self.screen_rect;
        if sr.width() <= 10.0 || sr.height() <= 10.0 {
            return false;
        }
        let side_panel_w = if sr.width() > 640.0 { 265.0 } else { 0.0 };
        let top_cut = self.top_bar_bottom_pt + 44.0;
        let bottom_cut = sr.bottom() - 84.0;
        pos.x > sr.left() + side_panel_w
            && pos.x < sr.right() - side_panel_w
            && pos.y > top_cut
            && pos.y < bottom_cut
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
                            let mouse_canvas_pan =
                                self.mouse_cursor_mode && self.is_inside_center_canvas(*pos);
                            if self.modifiers.space_pan || mouse_canvas_pan {
                                self.hand_pan_last_pos = Some(*pos);
                                self.long_press = None;
                                if mouse_canvas_pan {
                                    self.suppressed_touch_id = Some(*id);
                                    suppress_this_frame = true;
                                }
                            } else {
                                if let Some((prev_t, prev_pos)) = self.last_tap {
                                    if prev_t.elapsed() <= DOUBLE_TAP_WINDOW
                                        && prev_pos.distance(*pos) <= DOUBLE_TAP_SNAP_PT
                                    {
                                        snap_double_tap = Some(prev_pos);
                                        self.double_tap_snap = Some((*id, prev_pos));
                                    } else {
                                        self.double_tap_snap = None;
                                    }
                                } else {
                                    self.double_tap_snap = None;
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
                        if self.active_touches.len() == 1
                            && (self.modifiers.space_pan || self.mouse_cursor_mode)
                            && !palm_active
                        {
                            if let Some(prev) = self.hand_pan_last_pos {
                                hand_pan_delta += *pos - prev;
                                self.hand_pan_last_pos = Some(*pos);
                            } else if self.modifiers.space_pan && in_canvas {
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
                        if self.double_tap_snap.is_some_and(|(tid, _)| tid == *id) {
                            snap_double_tap = self.double_tap_snap.take().map(|(_, p)| p);
                        }
                    }
                }
            }
        }

        let active_snap = snap_double_tap.or_else(|| self.double_tap_snap.map(|(_, p)| p));
        if let Some(snap_pos) = active_snap {
            for ev in &mut raw_input.events {
                match ev {
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        ..
                    } => *pos = snap_pos,
                    egui::Event::PointerMoved(pos) => *pos = snap_pos,
                    _ => {}
                }
            }
        }
        // Keep egui's double-click detector alive between tap 1 and tap 2 (PointerGone resets it)
        if !self.mouse_cursor_mode
            && self.last_tap.is_some_and(|(t, _)| t.elapsed() <= DOUBLE_TAP_WINDOW)
        {
            raw_input.events.retain(|ev| !matches!(ev, egui::Event::PointerGone));
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

        if (self.modifiers.space_pan || self.mouse_cursor_mode)
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
                synthetic_events.push(egui::Event::PointerMoved(center));

                let start_dist = *self.pinch_start_dist.get_or_insert(dist);
                if !self.pinch_zoom_unlocked && (dist - start_dist).abs() >= PINCH_ZOOM_DEADZONE_PT {
                    self.pinch_zoom_unlocked = true;
                    self.last_pinch_dist = Some(dist);
                }

                if let Some(prev_center) = self.last_pinch_center {
                    let pan_delta = center - prev_center;
                    if pan_delta.length_sq() > 0.15 {
                        synthetic_events.push(egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: pan_delta,
                            phase: egui::TouchPhase::Move,
                            source: egui::MouseWheelSource::Unknown,
                            modifiers: active_modifiers,
                        });
                    }
                }

                if self.pinch_zoom_unlocked {
                    if let Some(prev_dist) = self.last_pinch_dist {
                        let zoom_ratio = (dist / prev_dist).clamp(0.5, 2.0);
                        if (zoom_ratio - 1.0).abs() >= PINCH_ZOOM_STEP_THRESHOLD {
                            synthetic_events.push(egui::Event::Zoom(zoom_ratio));
                            self.last_pinch_dist = Some(dist);
                        }
                    } else {
                        self.last_pinch_dist = Some(dist);
                    }
                }
                self.last_pinch_center = Some(center);
            }
        } else {
            self.pinch_start_dist = None;
            self.pinch_zoom_unlocked = false;
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

    /// Handles Grip-Circle Mouse Cursor Mode:
    /// - Only touches starting inside the bottom-right semi-transparent grip circle control the cursor
    /// - Tap grip: Left-click at cursor tip
    /// - Long-press grip (>= 500ms): Right-click at cursor tip
    /// - Drag grip: Move cursor + grip smoothly (hover aim)
    /// - Double-tap & hold grip: Left-button drag at cursor tip
    /// - Touches outside the grip circle (top bar, canvas, 2-finger pan/zoom) pass through normally and keep cursor stationary
    fn process_mouse_cursor_mode(
        &mut self,
        raw_input: &mut egui::RawInput,
        active_modifiers: egui::Modifiers,
        synthetic_events: &mut Vec<egui::Event>,
    ) -> bool {
        let grip_center = self.mouse_grip_center();
        let grip_hit_r = self.mouse_grip_radius() + 10.0;
        let mut grip_handled_this_frame = self.mouse_track.is_some();

        for event in &raw_input.events {
            if let egui::Event::Touch { id, phase, pos, .. } = event {
                match phase {
                    egui::TouchPhase::Start => {
                        if self.mouse_track.is_none()
                            && self.active_touches.is_empty()
                            && pos.distance(grip_center) <= grip_hit_r
                        {
                            grip_handled_this_frame = true;
                            let is_double_tap = self.last_mouse_tap.is_some_and(|(t, prev_f)| {
                                t.elapsed() <= MOUSE_DOUBLE_TAP_WINDOW
                                    && prev_f.distance(*pos) <= grip_hit_r + 16.0
                            });
                            self.mouse_track = Some(MouseTouchTrack {
                                touch_id: *id,
                                down_finger_pos: *pos,
                                last_finger_pos: *pos,
                                down_at: Instant::now(),
                                moved_far: false,
                                is_dragging: is_double_tap,
                                right_clicked: false,
                            });
                            let cursor = self.virtual_cursor_pos;
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
                        } else if self.mouse_track.is_some() {
                            // Second finger touched down while holding grip: cancel grip drag & hand off cleanly
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
                            grip_handled_this_frame = false;
                        }
                    }
                    egui::TouchPhase::Move => {
                        if let Some(tr) = &mut self.mouse_track {
                            if tr.touch_id == *id {
                                grip_handled_this_frame = true;
                                if tr.down_finger_pos.distance(*pos) > MOUSE_TAP_SLOP_PT {
                                    tr.moved_far = true;
                                }
                                if !tr.right_clicked {
                                    let delta = *pos - tr.last_finger_pos;
                                    tr.last_finger_pos = *pos;
                                    let max_x = self.screen_rect.right().max(100.0) - 1.0;
                                    let max_y = self.screen_rect.bottom().max(100.0) - 1.0;
                                    self.virtual_cursor_pos = egui::pos2(
                                        (self.virtual_cursor_pos.x + delta.x).clamp(0.0, max_x),
                                        (self.virtual_cursor_pos.y + delta.y).clamp(0.0, max_y),
                                    );
                                    synthetic_events.push(egui::Event::PointerMoved(self.virtual_cursor_pos));
                                }
                            }
                        }
                    }
                    egui::TouchPhase::End | egui::TouchPhase::Cancel => {
                        if let Some(tr) = self.mouse_track {
                            if tr.touch_id == *id {
                                grip_handled_this_frame = true;
                                self.mouse_track = None;
                                let cursor = self.virtual_cursor_pos;
                                if tr.is_dragging {
                                    synthetic_events.push(egui::Event::PointerButton {
                                        pos: cursor,
                                        button: egui::PointerButton::Primary,
                                        pressed: false,
                                        modifiers: active_modifiers,
                                    });
                                } else if !tr.right_clicked
                                    && matches!(phase, egui::TouchPhase::End)
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

        // Long-press on grip circle (>= 500ms without dragging) -> Right-click at cursor tip
        if let Some(tr) = &mut self.mouse_track {
            if !tr.moved_far
                && !tr.is_dragging
                && !tr.right_clicked
                && tr.down_at.elapsed() >= Duration::from_millis(500)
            {
                tr.right_clicked = true;
                let cursor = self.virtual_cursor_pos;
                synthetic_events.push(egui::Event::PointerMoved(cursor));
                synthetic_events.push(egui::Event::PointerButton {
                    pos: cursor,
                    button: egui::PointerButton::Secondary,
                    pressed: true,
                    modifiers: active_modifiers,
                });
                synthetic_events.push(egui::Event::PointerButton {
                    pos: cursor,
                    button: egui::PointerButton::Secondary,
                    pressed: false,
                    modifiers: active_modifiers,
                });
            }
        }

        if !grip_handled_this_frame {
            // Touch was NOT on the grip circle: let normal direct touch & 2-finger pan/zoom run while cursor stays still!
            return false;
        }

        // Strip raw finger touch/pointer events ONLY when the grip circle owns this touch
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

    /// Draws the user's custom rounded vector mouse cursor (Frame 1.svg, hotspot at 4,4)
    /// and the Ellipse.svg touch grip circle with adjustable ring opacity, X/Y offset, and border width.
    pub fn paint_virtual_cursor(&self, ctx: &egui::Context) {
        if !self.mouse_cursor_mode || self.is_palm_rejection_active() {
            return;
        }
        const CURSOR_PTS: &[(f32, f32)] = &[
    (-1.12, 15.4),
    (-1.15, 6.39),
    (-1.16, 4.85),
    (-1.14, 3.56),
    (-1.08, 2.5),
    (-0.966, 1.63),
    (-0.77, 0.942),
    (-0.476, 0.395),
    (-0.145, -0.000358),
    (0.235, -0.342),
    (0.657, -0.627),
    (1.11, -0.85),
    (1.6, -1.01),
    (2.11, -1.1),
    (2.73, -1.08),
    (3.43, -0.902),
    (4.23, -0.571),
    (5.18, -0.0891),
    (6.31, 0.541),
    (7.64, 1.31),
    (15.4, 5.85),
    (16.9, 6.71),
    (18.1, 7.44),
    (19.1, 8.06),
    (19.9, 8.58),
    (20.4, 9.02),
    (20.7, 9.42),
    (21.2, 10.4),
    (21.3, 11.3),
    (21.2, 12.3),
    (20.8, 13.2),
    (20.2, 14.0),
    (19.4, 14.6),
    (18.9, 14.8),
    (18.2, 14.9),
    (17.3, 15.0),
    (16.2, 15.1),
    (14.7, 15.1),
    (13.0, 15.1),
    (12.5, 15.2),
    (12.0, 15.2),
    (11.6, 15.2),
    (11.3, 15.2),
    (11.0, 15.3),
    (10.7, 15.4),
    (10.5, 15.4),
    (10.3, 15.5),
    (10.1, 15.7),
    (9.86, 15.8),
    (9.66, 15.9),
    (9.47, 16.1),
    (9.27, 16.3),
    (9.08, 16.5),
    (8.88, 16.8),
    (8.66, 17.1),
    (8.42, 17.5),
    (8.14, 18.0),
    (7.25, 19.4),
    (6.5, 20.6),
    (5.86, 21.6),
    (5.33, 22.4),
    (4.87, 22.9),
    (4.48, 23.2),
    (3.53, 23.6),
    (2.55, 23.8),
    (1.58, 23.6),
    (0.685, 23.2),
    (-0.0899, 22.6),
    (-0.686, 21.8),
    (-0.857, 21.3),
    (-0.974, 20.6),
    (-1.05, 19.7),
    (-1.09, 18.5),
    (-1.11, 17.1)
        ];
        const CURSOR_TRIS: &[u32] = &[73, 0, 1, 73, 1, 2, 73, 2, 3, 73, 3, 4, 73, 4, 5, 73, 5, 6, 73, 6, 7, 73, 7, 8, 73, 8, 9, 73, 9, 10, 73, 10, 11, 73, 11, 12, 73, 12, 13, 73, 13, 14, 73, 14, 15, 73, 15, 16, 73, 16, 17, 73, 17, 18, 73, 18, 19, 73, 19, 20, 73, 20, 21, 73, 21, 22, 73, 22, 23, 73, 23, 24, 73, 24, 25, 73, 25, 26, 73, 26, 27, 73, 27, 28, 73, 28, 29, 73, 29, 30, 30, 31, 32, 30, 32, 33, 30, 33, 34, 30, 34, 35, 30, 35, 36, 30, 36, 37, 30, 37, 38, 30, 38, 39, 30, 39, 40, 30, 40, 41, 30, 41, 42, 73, 30, 42, 73, 42, 43, 73, 43, 44, 73, 44, 45, 73, 45, 46, 73, 46, 47, 73, 47, 48, 73, 48, 49, 73, 49, 50, 73, 50, 51, 73, 51, 52, 73, 52, 53, 73, 53, 54, 73, 54, 55, 73, 55, 56, 73, 56, 57, 73, 57, 58, 73, 58, 59, 73, 59, 60, 73, 60, 61, 73, 61, 62, 73, 62, 63, 73, 63, 64, 73, 64, 65, 73, 65, 66, 73, 66, 67, 73, 67, 68, 73, 68, 69, 73, 69, 70, 73, 70, 71, 71, 72, 73];

        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            egui::Id::unique("figcraft_virtual_mouse_cursor"),
        ));
        let p = self.virtual_cursor_pos;
        let sc = self.mouse_cursor_scale.clamp(0.6, 2.0);
        let bw = self.mouse_border_width.clamp(0.5, 5.0);
        let ring_opacity = self.mouse_grip_opacity.clamp(0.05, 1.0);
        let dragging = self.mouse_track.is_some_and(|t| t.is_dragging);
        let active_grip = self.mouse_track.is_some();

        // 1. Touch grip circle (Ellipse.svg): ONLY the ring uses `ring_opacity`
        let grip_c = self.mouse_grip_center();
        let grip_r = self.mouse_grip_radius();
        let fill_alpha = ((ring_opacity * if active_grip { 1.2 } else { 1.0 }).clamp(0.05, 1.0) * 255.0).round() as u8;
        let stroke_alpha = ((ring_opacity * 1.45).clamp(0.2, 1.0) * 255.0).round() as u8;
        let grip_fill = if dragging {
            egui::Color32::from_rgba_unmultiplied(234, 88, 12, fill_alpha)
        } else {
            egui::Color32::from_rgba_unmultiplied(0, 0, 0, fill_alpha)
        };
        let grip_stroke_color = if dragging {
            egui::Color32::from_rgba_unmultiplied(253, 186, 116, stroke_alpha)
        } else {
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, stroke_alpha)
        };
        painter.circle(
            grip_c,
            grip_r,
            grip_fill,
            egui::Stroke::new(bw, grip_stroke_color),
        );

        // 2. Custom rounded mouse cursor (Frame 1.svg, hotspot at (4,4)) — always 100% opaque
        let mut mesh = egui::epaint::Mesh::default();
        let mut contour = Vec::with_capacity(CURSOR_PTS.len());
        for &(x, y) in CURSOR_PTS {
            let pos = p + egui::vec2(x * sc, y * sc);
            contour.push(pos);
            mesh.colored_vertex(pos, egui::Color32::WHITE);
        }
        mesh.indices.extend_from_slice(CURSOR_TRIS);
        painter.add(egui::Shape::mesh(mesh));

        // Crisp outer black border around the rounded cursor
        painter.add(egui::Shape::closed_line(
            contour,
            egui::Stroke::new(bw, egui::Color32::from_rgb(17, 17, 17)),
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
                        ui.set_height(btn_h);
                        ui.horizontal_centered(|ui| {
                            ui.spacing_mut().item_spacing = egui::vec2(5.0, 0.0);

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
                                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                                .show(ui, |ui| {
                                    ui.set_height(btn_h);
                                    ui.horizontal_centered(|ui| {
                                        ui.spacing_mut().item_spacing = egui::vec2(5.0, 0.0);
                                        if ui
                                            .add_sized(egui::vec2(44.0, btn_h), egui::Button::new("Shift").selected(self.modifiers.shift))
                                            .clicked()
                                        {
                                            self.modifiers.shift = !self.modifiers.shift;
                                        }
                                        if ui
                                            .add_sized(egui::vec2(42.0, btn_h), egui::Button::new("Ctrl").selected(self.modifiers.ctrl))
                                            .clicked()
                                        {
                                            self.modifiers.ctrl = !self.modifiers.ctrl;
                                        }
                                        if ui
                                            .add_sized(egui::vec2(38.0, btn_h), egui::Button::new("Alt").selected(self.modifiers.alt))
                                            .clicked()
                                        {
                                            self.modifiers.alt = !self.modifiers.alt;
                                        }
                                        if ui
                                            .add_sized(egui::vec2(50.0, btn_h), egui::Button::new("손바닥").selected(self.modifiers.space_pan))
                                            .clicked()
                                        {
                                            self.modifiers.space_pan = !self.modifiers.space_pan;
                                        }
                                        if ui
                                            .add_sized(egui::vec2(50.0, btn_h), egui::Button::new("그리기").selected(!self.long_press_enabled))
                                            .clicked()
                                        {
                                            self.long_press_enabled = !self.long_press_enabled;
                                            if !self.long_press_enabled {
                                                self.long_press = None;
                                            }
                                        }
                                        if ui
                                            .add_sized(egui::vec2(50.0, btn_h), egui::Button::new("마우스").selected(self.mouse_cursor_mode))
                                            .clicked()
                                        {
                                            actions.toggle_mouse = true;
                                        }
                                        ui.separator();
                                        if ui.add_sized(egui::vec2(58.0, btn_h), egui::Button::new("붙여넣기")).clicked() {
                                            actions.paste = true;
                                        }
                                        if ui.add_sized(egui::vec2(58.0, btn_h), egui::Button::new("실행취소")).clicked() {
                                            actions.undo = true;
                                        }
                                        if ui.add_sized(egui::vec2(58.0, btn_h), egui::Button::new("다시실행")).clicked() {
                                            actions.redo = true;
                                        }
                                        if ui.add_sized(egui::vec2(50.0, btn_h), egui::Button::new("키보드")).clicked() {
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
