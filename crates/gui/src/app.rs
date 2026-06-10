use iced::widget::{checkbox, column, container, row};
use std::sync::Arc;

use iced::{Element, Length, Subscription, Task};
use protocol::{ControlRef, GuiToDriver};
use settings::{PartialSettings, Settings};

use crate::device::hotspots::Device;
use crate::device::view::device_view;
use crate::message::Message;

pub struct State {
    pub(crate) status: String,
    /// Shared so the per-frame device overlay clones a pointer, not the whole
    /// nested settings tree.
    pub(crate) settings: Option<Arc<Settings>>,
    /// Last time a MIDI In / Out event arrived, for the activity LEDs.
    pub(crate) last_in: Option<std::time::Instant>,
    pub(crate) last_out: Option<std::time::Instant>,
    pub(crate) sender: Option<std::sync::mpsc::Sender<GuiToDriver>>,
    pub(crate) device_connected: bool,
    pub(crate) device: std::sync::Arc<Device>,
    pub(crate) selection: Vec<ControlRef>,
    pub(crate) touch_select: bool,
    pub(crate) show_prefs: bool,
    pub(crate) seq: u64,
    /// Highest apply `seq` the driver has acked. A pushed `Settings` snapshot is
    /// adopted as the live view only when this has caught up to `seq` — otherwise
    /// a snapshot for an older apply would clobber a newer optimistic edit.
    pub(crate) last_acked_seq: u64,
    /// Which numeric field is being typed, and its in-progress text.
    pub(crate) edit_field: Option<crate::inspector::assign::numeric::EditField>,
    pub(crate) edit_text: String,
    /// Active sub-action tab for the current selection.
    pub(crate) assign_tab: crate::inspector::assign::forms::AssignTab,
    /// Overlay every control's assignment label on the diagram.
    pub(crate) show_all_labels: bool,
    /// Last driver-confirmed settings (from a pushed `Settings` snapshot). Used
    /// to roll back an optimistic edit the driver rejected (`Ack(Err)`).
    pub(crate) authoritative: Option<Arc<Settings>>,
    /// Set when a rejected apply requested a resync while a newer edit was in
    /// flight: the next `Settings` snapshot must be adopted to clear the stale
    /// optimistic value even though `seq` has advanced past the rejected apply.
    pub(crate) resync_pending: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            status: String::new(),
            settings: None,
            last_in: None,
            last_out: None,
            sender: None,
            device_connected: false,
            device: std::sync::Arc::new(Device::load()),
            selection: Vec::new(),
            touch_select: true,
            show_prefs: false,
            seq: 0,
            last_acked_seq: 0,
            edit_field: None,
            edit_text: String::new(),
            assign_tab: crate::inspector::assign::forms::AssignTab::A,
            show_all_labels: false,
            authoritative: None,
            resync_pending: false,
        }
    }
}

impl State {
    pub fn new() -> Self {
        Self {
            status: "connecting…".to_string(),
            ..Self::default()
        }
    }

    pub fn title(&self) -> String {
        "Maschine Mikro MK3 — Configuration".to_string()
    }

    /// Optimistically merge `delta` into the local snapshot and send it to the
    /// driver. The driver validates, applies, persists, and pushes an
    /// authoritative `Settings` snapshot on success (or `Ack(Err)` on failure,
    /// which we surface and resync from).
    pub(crate) fn send_apply(&mut self, delta: PartialSettings, persist: bool) {
        let Some(sender) = &self.sender else { return };
        self.seq += 1;
        let _ = sender.send(GuiToDriver::Apply {
            seq: self.seq,
            delta: Box::new(delta.clone()),
            persist,
        });
        // Merge in place behind the shared Arc: make_mut clones the settings tree
        // only when the per-frame device overlay is still holding it, never rebuilds
        // the Arc, and a non-shared optimistic edit allocates nothing.
        if let Some(settings) = self.settings.as_mut() {
            let s = Arc::make_mut(settings);
            *s = std::mem::take(s).merge_overrides(delta);
        }
    }

    /// Internal indices of selected pads (empty if the selection isn't pads).
    pub(crate) fn selected_pads(&self) -> Vec<u8> {
        self.selection
            .iter()
            .filter_map(|c| match c {
                ControlRef::Pad(i) => Some(*i),
                _ => None,
            })
            .collect()
    }

    pub(crate) fn selected_buttons(&self) -> Vec<u8> {
        self.selection
            .iter()
            .filter_map(|c| match c {
                ControlRef::Button(i) => Some(*i),
                _ => None,
            })
            .collect()
    }

    pub(crate) fn reset_assign_edit(&mut self) {
        self.assign_tab = crate::inspector::assign::forms::AssignTab::A;
        self.edit_field = None;
        self.edit_text.clear();
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        crate::update::update(self, message)
    }

    pub fn view(&self) -> Element<'_, Message> {
        let top_bar = crate::shell::view::top_bar(self);
        let inspector = crate::inspector::view::inspector(self);

        let device_pane = container(
            column![
                container(
                    checkbox(self.show_all_labels)
                        .label("Show all labels")
                        .on_toggle(Message::ToggleShowAllLabels),
                )
                .width(Length::Fill)
                .align_x(iced::alignment::Horizontal::Right),
                container(device_view(self)).height(Length::Fill),
            ]
            .spacing(6),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_y(iced::alignment::Vertical::Top)
        .padding(8);

        let main = row![device_pane, inspector].height(Length::Fill);
        let base = column![top_bar, main].spacing(4);

        if self.show_prefs && self.settings.is_some() {
            return iced::widget::stack![base, crate::prefs::view::prefs_overlay(self)].into();
        }
        base.into()
    }

    /// The connection subscription auto-reconnects: `driver_connection` loops
    /// internally, reconnecting with backoff after the link drops, so the GUI
    /// recovers when the driver restarts without restarting the GUI itself.
    pub fn subscription(&self) -> Subscription<Message> {
        let mut subs = vec![Subscription::run(
            crate::io::subscription::driver_connection,
        )];
        // Drive the ~8Hz redraw timer only while an activity LED is (or just was)
        // lit, so an idle GUI does zero periodic redraws. Each MidiActivity
        // message re-evaluates this subscription and turns the timer back on; the
        // extra tick interval lets the final off-frame render before it stops.
        let now = std::time::Instant::now();
        let recent = |t: Option<std::time::Instant>| {
            t.is_some_and(|t| {
                now.duration_since(t).as_millis() < crate::shell::view::ACTIVITY_WINDOW_MS + 120
            })
        };
        if recent(self.last_in) || recent(self.last_out) {
            subs.push(
                iced::time::every(std::time::Duration::from_millis(120)).map(|_| Message::Tick),
            );
        }
        Subscription::batch(subs)
    }
}
