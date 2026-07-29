use driver::backend::midi::event_to_midi_bytes;
use driver::events::ControlEvent;
use driver::paging::{PagingAction, PagingState};
use driver::runtime_state::RuntimeState;
use driver::settings::Settings;
use driver::settings::actions::PadHitAction;
use maschine_library::controls::Buttons;

fn note_delta_for_page(page_active: bool) -> Settings {
    // Page 0: pad 0 → note 60. Page 1: pad 0 → note 72.
    let mut s = Settings::default();
    s.pad_paging.enabled = true;
    s.pad_paging.pages.push(s.pad_paging.new_page());
    s.pad_paging.pages[0].pads.0[0].hit = PadHitAction::Note {
        channel: None,
        note: 60,
    };
    s.pad_paging.pages[1].pads.0[0].hit = PadHitAction::Note {
        channel: None,
        note: 72,
    };
    s.pad_paging.active = if page_active { 1 } else { 0 };
    s
}

#[test]
fn active_page_selects_the_pad_note() {
    let rt = RuntimeState::default();
    let on = ControlEvent::PadNoteOn {
        index: 0,
        velocity: 100,
    };

    // Assert only the note byte, so the test does not depend on how an unset
    // channel resolves to a status byte.
    let page0 = note_delta_for_page(false);
    assert_eq!(event_to_midi_bytes(&on, &page0, &rt).unwrap()[1], 60);

    let page1 = note_delta_for_page(true);
    assert_eq!(
        event_to_midi_bytes(&on, &page1, &rt).unwrap()[1],
        72,
        "switching the active page routes the pad to the other page's note"
    );
}

#[test]
fn group_hold_pad_tap_yields_the_target_page() {
    // The state machine drives the active-page selection the loop then applies.
    let mut paging = PagingState::new();
    let group_press = ControlEvent::ButtonChanged {
        index: Buttons::Group as usize,
        pressed: true,
    };
    let group_release = ControlEvent::ButtonChanged {
        index: Buttons::Group as usize,
        pressed: false,
    };
    // Physical pad 2 selects page 2 (index 1).
    let tap = ControlEvent::PadNoteOn {
        index: settings::pads_by_index::config_key_to_internal(2),
        velocity: 100,
    };

    assert_eq!(
        paging.observe_event(0, 2, &group_press),
        PagingAction::OpenPicker
    );
    assert_eq!(
        paging.observe_event(0, 2, &tap),
        PagingAction::SelectPage(1)
    );
    assert_eq!(
        paging.observe_event(0, 2, &group_release),
        PagingAction::ClosePicker(1)
    );
}
