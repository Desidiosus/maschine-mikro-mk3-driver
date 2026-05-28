#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlEvent {
    ButtonChanged { index: usize, pressed: bool },
    EncoderTurn { delta: i8 },
    SliderMoved { raw: u8, cc_value: u8 },
    SliderTouch { pressed: bool },
    PadNoteOn { index: usize, velocity: u8 },
    PadNoteOff { index: usize, velocity: u8 },
    PadAftertouch { index: usize, pressure: u8 },
}
