use std::sync::{Arc, OnceLock};

use iced::alignment::Vertical;
use iced::widget::{container, svg, text};
use iced::{Color, Element, Length, Point, Rectangle, Renderer, Size, Theme, keyboard, mouse};
use protocol::ControlRef;
use settings::Settings;

use crate::app::State;
use crate::device::hotspots::{DEVICE_SVG, Device, Rect, device_transform};
use crate::device::labels::control_label;
use crate::message::Message;
use iced::widget::canvas::{self, Canvas, Frame, Geometry, Path, Stroke, Text};
/// Pixels of pointer travel before a press is treated as a drag (vs a click).
const DRAG_THRESHOLD: f32 = 4.0;
/// Fixed colour of the hardware-area frame drawn around the pad grid. The
/// reference editor uses the same muted blue-grey regardless of the active
/// page's colour, so this is a constant rather than derived from `pad_paging`.
const PAD_FRAME_COLOR: Color = Color::from_rgb(0.42, 0.55, 0.68);
/// Outset (device units, pre-scale) between the pad rects and the hardware
/// frame.
const PAD_FRAME_OUTSET: f32 = 14.0;

/// The hardware-area frame around the pad grid, in canvas coordinates for a
/// `width`×`height` device pane. `None` when the SVG exposed no pad ids.
///
/// The frame is drawn by the device canvas and the page selector is anchored
/// inside it from `app::State::view`; both resolve it here so the selector can
/// never drift off the rectangle that was actually drawn.
pub(crate) fn pad_frame_rect(device: &Device, width: f32, height: f32) -> Option<Rectangle> {
    let grid = device.pad_grid_rect()?;
    let (ox, oy, scale) = device_transform(width, height, device.size.0, device.size.1);
    Some(Rectangle {
        x: ox + (grid.x - PAD_FRAME_OUTSET) * scale,
        y: oy + (grid.y - PAD_FRAME_OUTSET) * scale,
        width: (grid.w + 2.0 * PAD_FRAME_OUTSET) * scale,
        height: (grid.h + 2.0 * PAD_FRAME_OUTSET) * scale,
    })
}
/// Whether a driver-supplied `ControlRef` index is in range for the fixed-size
/// settings arrays the inspector indexes (16 pads, 41 buttons).
pub(crate) fn control_index_valid(control: ControlRef) -> bool {
    match control {
        ControlRef::Pad(i) => (i as usize) < 16,
        ControlRef::Button(i) => (i as usize) < 41,
        ControlRef::Encoder | ControlRef::Slider => true,
    }
}

/// Whether the pointer moved far enough from the press point to count as a drag
/// (rather than a click). Single source of truth for both selection and marquee.
fn is_drag(start: Point, current: Point) -> bool {
    (start.x - current.x).abs() > DRAG_THRESHOLD || (start.y - current.y).abs() > DRAG_THRESHOLD
}

/// The marquee rect in device coords, or `None` if the pointer hasn't moved past
/// the click threshold. Shared by input (final selection) and drawing (highlight)
/// so both agree on what a drag covers.
fn drag_rect_device(start: Point, current: Point, ox: f32, oy: f32, scale: f32) -> Option<Rect> {
    if !is_drag(start, current) {
        return None;
    }
    let to_device = |p: Point| ((p.x - ox) / scale, (p.y - oy) / scale);
    let (sx, sy) = to_device(start);
    let (cx, cy) = to_device(current);
    Some(Rect {
        x: sx.min(cx),
        y: sy.min(cy),
        w: (sx - cx).abs(),
        h: (sy - cy).abs(),
    })
}

/// The controls whose label chips render as the active selection.
///
/// While a drag is in progress, the committed selection is hidden and only the
/// live marquee hits show — even when the marquee currently covers nothing, so
/// the old chips vanish the moment the drag starts. The committed selection is
/// restored only once the drag ends (`dragging` is false): a release that hit
/// nothing publishes no message, leaving the prior selection intact for the
/// next, non-dragging redraw.
fn active_selection(
    selection: &[ControlRef],
    drag_hits: &[ControlRef],
    dragging: bool,
) -> Vec<ControlRef> {
    if dragging {
        drag_hits.to_vec()
    } else {
        selection.to_vec()
    }
}

/// Image-free overlay canvas drawn ON TOP of the device picture. iced composites
/// images above vector fills, so the border, label chips, and marquee must live
/// on this picture-free canvas to render above the device. It owns all diagram
/// input: click, Ctrl+click, and drag-select.
pub(crate) struct DeviceCanvas {
    pub(crate) device: Arc<Device>,
    pub(crate) selection: Vec<ControlRef>,
    pub(crate) settings: Option<Arc<Settings>>,
    pub(crate) show_all_labels: bool,
    /// Every hotspot's label text, precomputed once when the canvas is built (only
    /// when `show_all_labels`). `draw` runs many times per build during a marquee
    /// drag, so computing labels here keeps the per-redraw work to a string copy
    /// instead of re-deriving every control's label from settings each frame.
    pub(crate) all_labels: Vec<(ControlRef, String)>,
}

#[derive(Default)]
pub(crate) struct SelectState {
    start: Option<Point>,
    current: Option<Point>,
    modifiers: keyboard::Modifiers,
    /// Controls the in-progress marquee covers, recomputed once per pointer move
    /// (not per redraw) and reused for both the live highlight and the final
    /// selection on release.
    drag_hits: Vec<ControlRef>,
}

impl canvas::Program<Message> for DeviceCanvas {
    type State = SelectState;

    fn update(
        &self,
        state: &mut SelectState,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let (ox, oy, scale) = device_transform(
            bounds.width,
            bounds.height,
            self.device.size.0,
            self.device.size.1,
        );
        if scale <= 0.0 {
            return None;
        }
        match event {
            canvas::Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => {
                state.modifiers = *m;
                None
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let pos = cursor.position_in(bounds)?;
                state.start = Some(pos);
                state.current = Some(pos);
                state.drag_hits.clear();
                Some(canvas::Action::capture())
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if state.start.is_some() {
                    state.current = cursor.position_in(bounds);
                    // Recompute the covered controls here (once per move) rather
                    // than on every redraw frame.
                    state.drag_hits = state
                        .start
                        .zip(state.current)
                        .and_then(|(s, c)| drag_rect_device(s, c, ox, oy, scale))
                        .map(|r| self.device.controls_in_rect(r))
                        .unwrap_or_default();
                    Some(canvas::Action::request_redraw())
                } else {
                    None
                }
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                // Only act on a press that began on this canvas. When something
                // above (e.g. the Preferences modal scrim) captured the press,
                // `start` is None; the release must not select, or clicks meant
                // for the overlay would fall through to the diagram.
                let Some(start) = state.start.take() else {
                    state.current = None;
                    state.drag_hits.clear();
                    return None;
                };
                state.current = None;
                let hits = std::mem::take(&mut state.drag_hits);
                let Some(pos) = cursor.position_in(bounds) else {
                    // Released off-canvas: abandon the gesture.
                    return None;
                };
                let msg = if is_drag(start, pos) {
                    // Reuse the hits computed during the drag so the committed
                    // selection matches the highlight the user just saw.
                    (!hits.is_empty()).then_some(Message::SelectControls(hits))
                } else {
                    let (dx, dy) = ((pos.x - ox) / scale, (pos.y - oy) / scale);
                    self.device.hit_test(dx, dy).map(|c| {
                        if state.modifiers.control() {
                            Message::ToggleControl(c)
                        } else {
                            Message::SelectControl(c)
                        }
                    })
                };
                Some(match msg {
                    Some(m) => canvas::Action::publish(m).and_capture(),
                    // No message means no app update to trigger a repaint, so the
                    // marquee from this gesture would linger; request a redraw to
                    // clear it now that `start`/`current` are reset.
                    None => canvas::Action::request_redraw().and_capture(),
                })
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        state: &SelectState,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let (ox, oy, scale) = device_transform(
            bounds.width,
            bounds.height,
            self.device.size.0,
            self.device.size.1,
        );

        // 1px black border framing the device picture (the shared transform
        // reserves DEVICE_INSET so this stroke has room on all four sides).
        let bw = 1.0_f32;
        let draw_w = (self.device.size.0 * scale).max(1.0);
        let draw_h = (self.device.size.1 * scale).max(1.0);
        let border = Path::rectangle(
            Point::new(ox - bw / 2.0, oy - bw / 2.0),
            Size::new(draw_w + bw, draw_h + bw),
        );
        frame.stroke(
            &border,
            Stroke::default().with_color(Color::BLACK).with_width(bw),
        );

        // Hardware-area frame around the 16-pad grid. This mirrors the official
        // Controller Editor, which shows this dashed frame at all times — paging
        // on or off — in a fixed colour that never changes with the active
        // page's colour, so the stroke below is a constant, not derived from
        // page state. It belongs to the device picture rather than to the
        // settings-derived overlay, so it is drawn before the no-settings early
        // return below: the GUI connects even when the driver is absent, and
        // that state can last indefinitely.
        if let Some(rect) = pad_frame_rect(&self.device, bounds.width, bounds.height) {
            let frame_rect = Path::rounded_rectangle(
                Point::new(rect.x, rect.y),
                Size::new(rect.width, rect.height),
                iced::border::Radius::from(4.0),
            );
            frame.stroke(
                &frame_rect,
                Stroke {
                    line_dash: canvas::LineDash {
                        segments: &[4.0, 4.0],
                        offset: 0,
                    },
                    ..Stroke::default()
                        .with_color(PAD_FRAME_COLOR)
                        .with_width(2.0)
                },
            );
        }

        let Some(settings) = &self.settings else {
            return vec![frame.into_geometry()];
        };

        // Controls the in-progress drag would select, for live highlight. Already
        // computed in `update` on the last pointer move — reused here, not redone.
        let highlight = &state.drag_hits;
        let dragging = state
            .start
            .zip(state.current)
            .is_some_and(|(s, c)| is_drag(s, c));

        // Scope the chip closure's `&mut frame` borrow so the marquee fill below
        // can borrow `frame` again.
        {
            let font_size = (15.0 * scale).clamp(11.0, 18.0);
            let box_w = 6.0 * font_size * 0.6 + 10.0;
            let box_h = font_size + 6.0;
            let mut draw_label = |control: ControlRef, label: String, selected: bool| {
                let Some(r) = self.device.rect_for(control) else {
                    return;
                };
                let cx = ox + (r.x + r.w / 2.0) * scale;
                let cy = oy + (r.y + r.h / 2.0) * scale;
                let fill = if selected {
                    Color::from_rgb(1.0, 0.749, 0.749)
                } else {
                    Color::from_rgb(0.878, 0.878, 0.878)
                };
                let rect = Path::rounded_rectangle(
                    Point::new(cx - box_w / 2.0, cy - box_h / 2.0),
                    Size::new(box_w, box_h),
                    iced::border::Radius::from(3.0),
                );
                frame.fill(&rect, fill);
                frame.stroke(
                    &rect,
                    Stroke::default().with_color(Color::BLACK).with_width(1.0),
                );
                frame.fill_text(Text {
                    content: label,
                    position: Point::new(cx, cy),
                    color: Color::from_rgb(0.1, 0.1, 0.1),
                    size: iced::Pixels(font_size),
                    align_x: text::Alignment::Center,
                    align_y: Vertical::Center,
                    ..Text::default()
                });
            };

            let active = active_selection(&self.selection, highlight, dragging);
            if self.show_all_labels {
                for (control, label) in &self.all_labels {
                    draw_label(*control, label.clone(), active.contains(control));
                }
            } else {
                for c in &active {
                    draw_label(*c, control_label(settings, *c), true);
                }
            }
        }

        // Marquee rectangle (canvas space), drawn above the chips.
        if let (Some(s), Some(c)) = (state.start, state.current)
            && is_drag(s, c)
        {
            let rect = Path::rectangle(
                Point::new(s.x.min(c.x), s.y.min(c.y)),
                Size::new((s.x - c.x).abs(), (s.y - c.y).abs()),
            );
            frame.fill(&rect, Color::from_rgba(0.40, 0.60, 1.0, 0.15));
            frame.stroke(
                &rect,
                Stroke::default()
                    .with_color(Color::from_rgb(0.40, 0.60, 1.0))
                    .with_width(1.0),
            );
        }
        vec![frame.into_geometry()]
    }
}

/// The device picture's parsed SVG handle, built once. `svg::Handle::from_memory`
/// hashes the whole ~340 KB asset to key its cache, so rebuilding it every frame
/// would re-hash the asset on each repaint; clone the shared handle instead.
fn device_svg_handle() -> svg::Handle {
    static HANDLE: OnceLock<svg::Handle> = OnceLock::new();
    HANDLE
        .get_or_init(|| svg::Handle::from_memory(DEVICE_SVG))
        .clone()
}

pub(crate) fn device_view(state: &State) -> Element<'_, Message> {
    // Bottom: the device picture as a vector svg, padded so its contain-fit box
    // matches `device_transform` (which reserves DEVICE_INSET for the border).
    // Top: the image-free input/overlay canvas.
    // The svg widget defaults to `height: Shrink`, which would top-align the
    // picture inside the Fill container while the overlay canvas centers the
    // device via `device_transform` — misaligning chips and hit-testing. Force
    // Fill on both axes so the svg's contain-fit box matches the canvas exactly.
    let picture = container(
        svg(device_svg_handle())
            .width(Length::Fill)
            .height(Length::Fill),
    );
    let all_labels = match (state.show_all_labels, &state.settings) {
        (true, Some(settings)) => state
            .device
            .hotspots
            .iter()
            .map(|h| (h.control, control_label(settings, h.control)))
            .collect(),
        _ => Vec::new(),
    };
    let overlay = Canvas::new(DeviceCanvas {
        device: state.device.clone(),
        selection: state.selection.clone(),
        settings: state.settings.clone(),
        show_all_labels: state.show_all_labels,
        all_labels,
    })
    .width(Length::Fill)
    .height(Length::Fill);
    iced::widget::stack![picture, overlay].into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_shows_committed_selection() {
        let selection = vec![ControlRef::Pad(0), ControlRef::Pad(1)];
        assert_eq!(active_selection(&selection, &[], false), selection);
    }

    #[test]
    fn active_drag_shows_only_live_hits() {
        let selection = vec![ControlRef::Pad(0)];
        let drag_hits = vec![ControlRef::Pad(5), ControlRef::Pad(6)];
        assert_eq!(active_selection(&selection, &drag_hits, true), drag_hits);
    }

    #[test]
    fn drag_over_nothing_hides_prior_selection() {
        let selection = vec![ControlRef::Pad(0), ControlRef::Button(3)];
        assert_eq!(active_selection(&selection, &[], true), Vec::new());
    }

    #[test]
    fn the_pad_frame_encloses_every_pad_at_any_pane_size() {
        let device = Device::load();
        for (w, h) in [(800.0, 500.0), (1600.0, 900.0), (400.0, 900.0)] {
            let frame = pad_frame_rect(&device, w, h).expect("the SVG exposes pad ids");
            let (ox, oy, scale) = device_transform(w, h, device.size.0, device.size.1);
            for i in 0..16u8 {
                let pad = device
                    .rect_for(ControlRef::Pad(i))
                    .expect("every pad has a hotspot");
                let (x, y) = (ox + pad.x * scale, oy + pad.y * scale);
                assert!(
                    x > frame.x
                        && y > frame.y
                        && x + pad.w * scale < frame.x + frame.width
                        && y + pad.h * scale < frame.y + frame.height,
                    "pad {i} must sit inside the frame at {w}x{h}"
                );
            }
        }
    }
}
