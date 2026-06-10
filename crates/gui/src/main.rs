use gui::app::State;

fn main() -> iced::Result {
    iced::application(State::new, State::update, State::view)
        .title(State::title)
        .subscription(State::subscription)
        .window(iced::window::Settings {
            size: iced::Size::new(1500.0, 820.0),
            min_size: Some(iced::Size::new(1280.0, 720.0)),
            ..Default::default()
        })
        .run()
}
