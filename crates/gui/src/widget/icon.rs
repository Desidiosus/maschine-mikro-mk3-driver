use iced::Length;

pub const SETTINGS_SVG: &[u8] = include_bytes!("../../assets/settings.svg");
pub const USB_SVG: &[u8] = include_bytes!("../../assets/usb.svg");
pub const USB_OFF_SVG: &[u8] = include_bytes!("../../assets/usb_off.svg");

/// A fixed-size, color-tinted SVG icon.
pub fn svg_icon<'a>(bytes: &'static [u8], color: iced::Color, size: f32) -> iced::widget::Svg<'a> {
    iced::widget::svg(iced::widget::svg::Handle::from_memory(bytes))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .style(move |_t: &iced::Theme, _s| iced::widget::svg::Style { color: Some(color) })
}
