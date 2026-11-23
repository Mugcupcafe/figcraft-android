//! Tablet touch, S Pen / USI stylus telemetry, multi-touch zoom/pan,
//! long-press context menu (550ms + drawing lock), finger double-tap snap,
//! Hand pan mode, and DPI-responsive Shortcut Pad (V6_hotfix2 · egui 0.37 native).

use crate::protocol::{ActivityEvent, StylusAction};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

const LONG_PRESS_MS: u64 = 550;
const DOUBLE_TAP_MS: u64 = 350;
const DOUBLE_TAP_RADIUS_PT: f32 = 24.0;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct VirtualModifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub space_pan: bool,
}

impl VirtualModifiers {
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

#[derive(Debug, Clone, Copy, Default)]
pub struct ShortcutPadActions {
    pub undo: bool,
    pub redo: bool,
    pub paste: bool,
    pub toggle_ime: bool,
}

#[derive(Debug, Clone)]
struct LongPressTracker {
    touch_id: egui::TouchId,
    origin: egui::Pos2,
    started_at: Instant,
    triggered: bool,
}

#[derive(Debug, Clone)]
pub struct TabletInputState {
    pub stylus_active: bool,
    pub last_stylus_pos_pt: egui::Pos2,
    pub last_pressure: f32,
    pub last_tilt: (f32, f32),
    pub is_eraser: bool,
    pub show_shortcut_pad: bool,
    pub long_press_enabled: bool,
    pub modifiers: VirtualModifiers,
    pub safe_insets_px: [f32; 4], // top, right, bottom, left
    pub ime_height_px: f32,
    pub pixels_per_point: f32,
    pub top_bar_bottom_pt: f32,
    pub shortcut_pad_rect: Option<egui::Rect>,
    pub canvas_rect: Option<egui::Rect>,

    active_touches: BTreeMap<egui::TouchId, egui::Pos2>,
    was_multi_touch: bool,
    last_pinch_dist: Option<f32>,
    last_pinch_center: Option<egui::Pos2>,
    hand_pan_last_pos: Option<egui::Pos2>,
    long_press: Option<LongPressTracker>,
    suppressed_touch_id: Option<egui::TouchId>,
    last_tap: Option<(Instant, egui::Pos2)>,
    last_sent_modifiers: egui::Modifiers,
}

impl Default for TabletInputState {
    fn default() -> Self {
        Self {
            stylus_active: false,
            last_stylus_pos_pt: egui::Pos2::ZERO,
            last_pressure: 0.0,
            last_tilt: (0.0, 0.0),
            is_eraser: false,
            show_shortcut_pad: true,
            long_press_enabled: true,
            modifiers: VirtualModifiers::default(),
            safe_insets_px: [0.0; 4],
            ime_height_px: 0.0,
            pixels_per_point: 1.0,
            top_bar_bottom_pt: 0.0,
            shortcut_pad_rect: None,
            canvas_rect: None,
            active_touches: BTreeMap::new(),
            was_multi_touch: false,
            last_pinch_dist: None,
            last_pinch_center: None,
            hand_pan_last_pos: None,
            long_press: None,
            suppressed_touch_id: None,
            last_tap: None,
            last_sent_modifiers: egui::Modifiers::NONE,
        }
    }
}

impl TabletInputState {
    pub fn set_pixels_per_point(&mut self, ppp: f32) {
        if ppp > 0.1 {
            self.pixels_per_point = ppp;
        }
    }

    pub fn handle_event(&mut self, event: &ActivityEvent) {
        match *event {
            ActivityEvent::WindowInsets {
                top,
                right,
                bottom,
                left,
            } => {
                self.safe_insets_px = [top, right, bottom, left];
            }
            ActivityEvent::ImeChanged { visible, height } => {
                self.ime_height_px = if visible { height } else { 0.0 };
            }
            ActivityEvent::StylusSample {
                x,
                y,
                pressure,
                tilt_x,
                tilt_y,
                is_eraser,
                action,
            } => {
                let ppp = self.pixels_per_point.max(1.0);
                self.last_stylus_pos_pt = egui::pos2(x / ppp, y / ppp);
                self.last_pressure = pressure.clamp(0.0, 1.0);
                self.last_tilt = (tilt_x, tilt_y);
                self.is_eraser = is_eraser;
                self.stylus_active = !matches!(action, StylusAction::Up);
            }
            _ => {}
        }
    }

    pub fn needs_repaint(&self) -> bool {
        self.long_press_enabled && self.long_press.as_ref().is_some_and(|lp| !lp.triggered)
    }

    fn is_inside_canvas(&self, pos: egui::Pos2) -> bool {
        if pos.y <= self.top_bar_bottom_pt {
            return false;
        }
        if let Some(rect) = self.shortcut_pad_rect {
            if rect.contains(pos) {
                return false;
            }
        }
        if let Some(canvas) = self.canvas_rect {
            return canvas.contains(pos);
        }
        true
    }

    pub fn process_raw_input(&mut self, raw_input: &mut egui::RawInput) {
        let active_modifiers = self.modifiers.to_egui_modifiers();
        let mut synthetic_events: Vec<egui::Event> = Vec::new();

        if active_modifiers != self.last_sent_modifiers {
            synthetic_events.push(egui::Event::ModifiersChanged(active_modifiers));
            self.last_sent_modifiers = active_modifiers;
        }

        let mut released_suppressed = false;
        let mut hand_pan_delta = egui::Vec2::ZERO;
        let mut suppress_for_hand_pan = false;
        let mut snap_double_tap: Option<egui::Pos2> = None;

        for ev in &raw_input.events {
            if let egui::Event::Touch {
                id, phase, pos, ..
            } = *ev
            {
                let in_canvas = self.is_inside_canvas(pos);

                match phase {
                    egui::TouchPhase::Start => {
                        if in_canvas {
                            if let Some((prev_t, prev_pos)) = self.last_tap {
                                if prev_t.elapsed() <= Duration::from_millis(DOUBLE_TAP_MS)
                                    && prev_pos.distance(pos) <= DOUBLE_TAP_RADIUS_PT
                                {
                                    snap_double_tap = Some(prev_pos);
                                    self.last_tap = None;
                                } else {
                                    self.last_tap = Some((Instant::now(), pos));
                                }
                            } else {
                                self.last_tap = Some((Instant::now(), pos));
                            }

                            self.active_touches.insert(id, pos);
                            if self.active_touches.len() == 1 {
                                if self.modifiers.space_pan {
                                    self.hand_pan_last_pos = Some(pos);
                                    suppress_for_hand_pan = true;
                                } else if self.long_press_enabled {
                                    self.long_press = Some(LongPressTracker {
                                        touch_id: id,
                                        origin: pos,
                                        started_at: Instant::now(),
                                        triggered: false,
                                    });
                                }
                            } else {
                                self.long_press = None;
                                self.hand_pan_last_pos = None;
                            }
                        } else {
                            self.last_tap = None;
                        }
                    }
                    egui::TouchPhase::Move => {
                        if self.active_touches.contains_key(&id) {
                            self.active_touches.insert(id, pos);
                            if self.modifiers.space_pan && self.active_touches.len() == 1 {
                                if let Some(prev) = self.hand_pan_last_pos {
                                    hand_pan_delta += pos - prev;
                                }
                                self.hand_pan_last_pos = Some(pos);
                                suppress_for_hand_pan = true;
                            }
                            if let Some(lp) = &self.long_press {
                                if lp.touch_id == id && lp.origin.distance(pos) > 10.0 {
                                    self.long_press = None;
                                }
                            }
                        }
                    }
                    egui::TouchPhase::End | egui::TouchPhase::Cancel => {
                        self.active_touches.remove(&id);
                        if self.active_touches.len() < 2 {
                            self.last_pinch_dist = None;
                            self.last_pinch_center = None;
                        }
                        if self.active_touches.is_empty() {
                            self.hand_pan_last_pos = None;
                        }
                        if self
                            .long_press
                            .as_ref()
                            .is_some_and(|lp| lp.touch_id == id)
                        {
                            self.long_press = None;
                        }
                        if self.suppressed_touch_id == Some(id) {
                            released_suppressed = true;
                        }
                    }
                }
            }
        }

        if let Some(snapped) = snap_double_tap {
            for ev in &mut raw_input.events {
                if let egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    ..
                } = ev
                {
                    *pos = snapped;
                }
            }
        }

        if self.suppressed_touch_id.is_some() {
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
            if released_suppressed {
                self.suppressed_touch_id = None;
            }
        }

        if suppress_for_hand_pan || (self.modifiers.space_pan && self.active_touches.len() == 1) {
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
            if hand_pan_delta.length_sq() > 0.25 {
                synthetic_events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: hand_pan_delta,
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Unknown,
                    modifiers: active_modifiers,
                });
            }
        }

        let is_multi_touch = self.active_touches.len() >= 2;
        if is_multi_touch {
            if !self.was_multi_touch {
                if let Some(&first_pos) = self.active_touches.values().next() {
                    synthetic_events.push(egui::Event::PointerButton {
                        pos: first_pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: active_modifiers,
                    });
                }
                synthetic_events.push(egui::Event::PointerGone);
            }

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

            if self.active_touches.len() == 2 {
                let mut iter = self.active_touches.values();
                if let (Some(&p1), Some(&p2)) = (iter.next(), iter.next()) {
                    let dist = p1.distance(p2).max(1.0);
                    let center = egui::pos2((p1.x + p2.x) * 0.5, (p1.y + p2.y) * 0.5);

                    if let (Some(prev_dist), Some(prev_center)) =
                        (self.last_pinch_dist, self.last_pinch_center)
                    {
                        let zoom_ratio = (dist / prev_dist).clamp(0.75, 1.35);
                        if (zoom_ratio - 1.0).abs() > 0.004 {
                            synthetic_events.push(egui::Event::Zoom(zoom_ratio));
                        }
                        let pan_delta = center - prev_center;
                        if pan_delta.length_sq() > 0.25 {
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
            }
        }
        self.was_multi_touch = is_multi_touch;

        if self.long_press_enabled {
            if let Some(lp) = &mut self.long_press {
                if !lp.triggered && lp.started_at.elapsed() >= Duration::from_millis(LONG_PRESS_MS) {
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
        }

        raw_input.events.extend(synthetic_events);
    }

    pub fn show_shortcut_pad_ui(&mut self, ctx: &egui::Context) -> ShortcutPadActions {
        let mut actions = ShortcutPadActions::default();

        if !self.show_shortcut_pad {
            self.shortcut_pad_rect = None;
            return actions;
        }

        let ppp = self.pixels_per_point.max(1.0);
        let right_inset = self.safe_insets_px[1] / ppp;
        let bottom_margin = (self.safe_insets_px[2].max(self.ime_height_px) / ppp) + 12.0;
        let left_margin = (self.safe_insets_px[3] / ppp) + 12.0;

        let screen_w = ctx.input(|i| i.viewport_rect()).width();
        let max_pad_w = (screen_w - left_margin - right_inset - 12.0).max(200.0);
        let btn_w = ((max_pad_w - 92.0) / 9.0).clamp(38.0, 48.0);
        let btn_size = egui::vec2(btn_w, 38.0);

        let area_resp = egui::Area::new(egui::Id::from("figcraft_tablet_shortcut_pad"))
            .anchor(
                egui::Align2::LEFT_BOTTOM,
                egui::vec2(left_margin, -bottom_margin),
            )
            .interactable(true)
            .show(ctx, |ui| {
                ui.set_max_width(max_pad_w);
                egui::Frame::window(ui.style())
                    .corner_radius(egui::CornerRadius::same(14))
                    .inner_margin(egui::Margin::symmetric(8, 6))
                    .show(ui, |ui| {
                        egui::ScrollArea::horizontal()
                            .id_salt("shortcut_pad_scroll")
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 6.0;
                                    if ui
                                        .add_sized(btn_size, egui::Button::new("Shift").selected(self.modifiers.shift))
                                        .clicked()
                                    {
                                        self.modifiers.shift = !self.modifiers.shift;
                                    }
                                    if ui
                                        .add_sized(btn_size, egui::Button::new("Ctrl").selected(self.modifiers.ctrl))
                                        .clicked()
                                    {
                                        self.modifiers.ctrl = !self.modifiers.ctrl;
                                    }
                                    if ui
                                        .add_sized(btn_size, egui::Button::new("Alt").selected(self.modifiers.alt))
                                        .clicked()
                                    {
                                        self.modifiers.alt = !self.modifiers.alt;
                                    }
                                    if ui
                                        .add_sized(btn_size, egui::Button::new("Hand").selected(self.modifiers.space_pan))
                                        .clicked()
                                    {
                                        self.modifiers.space_pan = !self.modifiers.space_pan;
                                    }
                                    if ui
                                        .add_sized(btn_size, egui::Button::new("Draw").selected(!self.long_press_enabled))
                                        .on_hover_text("Disable long-press menu while drawing shapes/paths")
                                        .clicked()
                                    {
                                        self.long_press_enabled = !self.long_press_enabled;
                                    }
                                    ui.separator();
                                    if ui.add_sized(btn_size, egui::Button::new("Paste")).clicked() {
                                        actions.paste = true;
                                    }
                                    if ui.add_sized(btn_size, egui::Button::new("Undo")).clicked() {
                                        actions.undo = true;
                                    }
                                    if ui.add_sized(btn_size, egui::Button::new("Redo")).clicked() {
                                        actions.redo = true;
                                    }
                                    if ui.add_sized(btn_size, egui::Button::new("KB")).clicked() {
                                        actions.toggle_ime = true;
                                    }
                                });
                            });
                    });
            });

        self.shortcut_pad_rect = Some(area_resp.response.rect);
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touches_outside_canvas_rect_are_never_blocked_by_hand_mode() {
        let mut state = TabletInputState::default();
        state.modifiers.space_pan = true;
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
        f1.events.push(egui::Event::PointerButton {
            pos: egui::pos2(50.0, 200.0),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        state.process_raw_input(&mut f1);

        assert!(f1
            .events
            .iter()
            .any(|e| matches!(e, egui::Event::PointerButton { pressed: true, .. })));
    }
}
