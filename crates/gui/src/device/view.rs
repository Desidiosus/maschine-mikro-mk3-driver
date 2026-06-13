use std::sync::{Arc, OnceLock};

use crate::app::State;
use crate::device::hotspots::{DEVICE_INSET, DEVICE_SVG, Device, device_transform};
use crate::message::Message;
use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke};
use iced::widget::{container, svg};
use iced::{Color, Element, Length, Point, Rectangle, Renderer, Size, Theme, mouse};

pub(crate) struct DeviceCanvas {
    pub(crate) device: Arc<Device>,
}

#[derive(Default)]
pub(crate) struct SelectState;

impl canvas::Program<Message> for DeviceCanvas {
    type State = SelectState;

    fn update(
        &self,
        _state: &mut SelectState,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let pos = cursor.position_in(bounds)?;
        // Use the SAME transform as draw() (including the border inset) so a
        // click maps to where the device is actually rendered.
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
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let (dx, dy) = ((pos.x - ox) / scale, (pos.y - oy) / scale);
                let msg = self.device.hit_test(dx, dy).map(Message::SelectControl);
                Some(match msg {
                    Some(m) => canvas::Action::publish(m).and_capture(),
                    None => canvas::Action::capture(),
                })
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &SelectState,
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
    )
    .padding(DEVICE_INSET)
    .width(Length::Fill)
    .height(Length::Fill);
    let overlay = canvas::Canvas::new(DeviceCanvas {
        device: state.device.clone(),
    })
    .width(Length::Fill)
    .height(Length::Fill);
    iced::widget::stack![picture, overlay].into()
}
