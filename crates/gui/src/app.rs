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
    pub(crate) device: Arc<Device>,
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
    /// Active top-level inspector tab (Assign vs Pages).
    pub(crate) inspector_tab: crate::message::InspectorTab,
    /// Overlay every control's assignment label on the diagram.
    pub(crate) show_all_labels: bool,
    /// Last driver-confirmed settings (from a pushed `Settings` snapshot). Used
    /// to roll back an optimistic edit the driver rejected (`Ack(Err)`).
    pub(crate) authoritative: Option<Arc<Settings>>,
    /// Set when a rejected apply requested a resync while a newer edit was in
    /// flight: the next `Settings` snapshot must be adopted to clear the stale
    /// optimistic value even though `seq` has advanced past the rejected apply.
    pub(crate) resync_pending: bool,
    /// Debounce countdown (in `PERSIST_DEBOUNCE_INTERVAL` ticks) for persisting
    /// typed numeric edits. Each keystroke applies live (`persist:false`) and
    /// re-arms this; when it counts down to zero the live state is persisted.
    /// Zero means no persist is pending.
    pub(crate) persist_debounce: u8,
    /// The page index a Delete-page confirmation dialog is open for, `None`
    /// when no dialog is showing. Deletion happens only when the dialog is
    /// confirmed, never directly from the row action button.
    pub(crate) confirm_delete_page: Option<usize>,
    /// The page row currently showing its `text_input` for renaming (see
    /// `Message::BeginRenamePage`), `None` when every row shows plain,
    /// non-editable text. Cleared on `SelectPage`, `CommitPageName`, and
    /// whenever an authoritative settings snapshot is adopted, so a stale
    /// index can never point at a row that moved or no longer exists.
    pub(crate) editing_page_name: Option<usize>,
}

/// In-place page-vec mutations, each clamping the paging invariants
/// (`MIN_PAGES..=MAX_PAGES` pages, `active` in range). Pure so they can be
/// unit-tested and reused inside `apply_pad_paging` closures.
pub(crate) mod page_ops {
    use settings::{MAX_PAGES, MIN_PAGES, PadPaging};

    /// The next default page name: "Pad Page A" through "Pad Page Z", the
    /// first letter no existing page already uses as its exact name. Assigned
    /// once at creation and never derived from position, so a reorder never
    /// renames. `MAX_PAGES` (16) < 26, so whenever a page can still be added
    /// a free letter exists; the numeric fallback is purely defensive.
    pub(crate) fn next_page_name(pp: &PadPaging) -> String {
        let taken: Vec<&str> = pp.pages.iter().filter_map(|p| p.name.as_deref()).collect();
        ('A'..='Z')
            .map(|letter| format!("Pad Page {letter}"))
            .find(|candidate| !taken.iter().any(|name| name == candidate))
            .unwrap_or_else(|| format!("Pad Page {}", pp.pages.len() + 1))
    }

    pub(crate) fn add(pp: &mut PadPaging) {
        if pp.pages.len() < MAX_PAGES {
            let name = next_page_name(pp);
            let mut page = settings::pad_paging::default_page();
            page.name = Some(name);
            pp.pages.push(page);
        }
    }

    pub(crate) fn duplicate(pp: &mut PadPaging, index: usize) {
        if pp.pages.len() < MAX_PAGES
            && let Some(mut page) = pp.pages.get(index).cloned()
        {
            // A duplicate is a new page, not a clone of the source's identity:
            // it gets its own fresh letter rather than reusing the source's name.
            page.name = Some(next_page_name(pp));
            pp.pages.insert(index + 1, page);
            // The copy lands at index+1, so an active page after that shifts up
            // one. Duplicating the active page itself keeps `active` on the
            // original, with the copy directly after it.
            if pp.active > index {
                pp.active += 1;
            }
        }
    }

    pub(crate) fn delete(pp: &mut PadPaging, index: usize) {
        if pp.pages.len() > MIN_PAGES && index < pp.pages.len() {
            pp.pages.remove(index);
            // Keep the same page active by content: a removal before the active
            // index shifts it down one. Removing the active page itself leaves
            // `active` on whichever page took its slot.
            if index < pp.active {
                pp.active -= 1;
            }
            if pp.active >= pp.pages.len() {
                pp.active = pp.pages.len() - 1;
            }
        }
    }
}

#[cfg(test)]
mod page_ops_tests {
    use super::page_ops::*;
    use settings::PadPaging;
    use settings::pad_paging::default_pad_paging;

    #[test]
    fn add_respects_max_16() {
        let mut pp = default_pad_paging();
        for _ in 0..20 {
            add(&mut pp);
        }
        assert_eq!(pp.pages.len(), 16);
    }

    #[test]
    fn delete_clamps_active_and_keeps_one_page() {
        let mut pp = default_pad_paging();
        add(&mut pp); // 2 pages
        pp.active = 1;
        delete(&mut pp, 1);
        assert_eq!(pp.pages.len(), 1);
        assert_eq!(pp.active, 0);
        delete(&mut pp, 0); // must not drop the last page
        assert_eq!(pp.pages.len(), 1);
    }

    #[test]
    fn duplicate_inserts_after_source() {
        let mut pp = default_pad_paging();
        pp.pages[0].name = Some("A".into());
        duplicate(&mut pp, 0);
        assert_eq!(pp.pages.len(), 2);
        // The copy is a new page, not a clone of the source's identity: it
        // gets its own fresh letter rather than reusing "A".
        assert_eq!(pp.pages[1].name.as_deref(), Some("Pad Page A"));
    }

    #[test]
    fn add_assigns_the_first_free_letter() {
        let mut pp = default_pad_paging();
        pp.pages[0].name = Some("Pad Page A".to_string());
        add(&mut pp);
        assert_eq!(pp.pages[1].name.as_deref(), Some("Pad Page B"));
        add(&mut pp);
        assert_eq!(pp.pages[2].name.as_deref(), Some("Pad Page C"));
    }

    #[test]
    fn add_reuses_a_letter_freed_by_delete() {
        let mut pp = default_pad_paging();
        pp.pages[0].name = Some("Pad Page A".to_string());
        add(&mut pp); // Pad Page B
        add(&mut pp); // Pad Page C
        delete(&mut pp, 1);
        add(&mut pp);
        assert_eq!(pp.pages[2].name.as_deref(), Some("Pad Page B"));
    }

    #[test]
    fn renamed_pages_do_not_block_letter_assignment() {
        let mut pp = default_pad_paging();
        pp.pages[0].name = Some("Kick".to_string());
        add(&mut pp);
        assert_eq!(pp.pages[1].name.as_deref(), Some("Pad Page A"));
    }

    #[test]
    fn duplicate_gives_the_copy_a_fresh_letter() {
        let mut pp = default_pad_paging();
        pp.pages[0].name = Some("Kick".to_string());
        duplicate(&mut pp, 0);
        assert_eq!(pp.pages[1].name.as_deref(), Some("Pad Page A"));
    }

    /// Three named pages so assertions are about page *identity*, not index.
    fn named_pages(names: [&str; 3]) -> PadPaging {
        let mut pp = default_pad_paging();
        add(&mut pp);
        add(&mut pp);
        for (page, name) in pp.pages.iter_mut().zip(names) {
            page.name = Some(name.to_string());
        }
        pp
    }

    #[test]
    fn delete_before_active_keeps_the_same_page_active() {
        let mut pp = named_pages(["A", "B", "C"]);
        pp.active = 1; // B
        delete(&mut pp, 0); // remove A, which sits before the active page
        assert_eq!(pp.active, 0);
        assert_eq!(pp.pages[pp.active].name.as_deref(), Some("B"));
    }

    #[test]
    fn deleting_the_active_page_lands_on_the_page_that_took_its_slot() {
        let mut pp = named_pages(["A", "B", "C"]);
        pp.active = 1; // B
        delete(&mut pp, 1); // remove the active page itself
        assert_eq!(pp.active, 1);
        assert_eq!(pp.pages[pp.active].name.as_deref(), Some("C"));
    }

    #[test]
    fn duplicate_before_active_keeps_the_same_page_active() {
        let mut pp = named_pages(["A", "B", "C"]);
        pp.active = 2; // C
        duplicate(&mut pp, 0); // insert a copy of A before the active page
        assert_eq!(pp.active, 3);
        assert_eq!(pp.pages[pp.active].name.as_deref(), Some("C"));
    }
}

/// Tick interval for the typed-edit persist debounce.
pub(crate) const PERSIST_DEBOUNCE_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(300);

/// Quiet ticks required after the last keystroke before a typed edit persists.
/// Two ticks guarantee at least one full quiet interval (the timer is free-running,
/// so a tick may arrive immediately after a keystroke).
pub(crate) const PERSIST_DEBOUNCE_TICKS: u8 = 2;

impl Default for State {
    fn default() -> Self {
        Self {
            status: String::new(),
            settings: None,
            last_in: None,
            last_out: None,
            sender: None,
            device_connected: false,
            device: Arc::new(Device::load()),
            selection: Vec::new(),
            touch_select: true,
            show_prefs: false,
            seq: 0,
            last_acked_seq: 0,
            edit_field: None,
            edit_text: String::new(),
            assign_tab: crate::inspector::assign::forms::AssignTab::A,
            inspector_tab: crate::message::InspectorTab::Assign,
            show_all_labels: false,
            authoritative: None,
            resync_pending: false,
            persist_debounce: 0,
            confirm_delete_page: None,
            editing_page_name: None,
        }
    }
}

impl State {
    pub fn new() -> Self {
        let prefs = crate::prefs::persistence::GuiPrefs::load();
        Self {
            status: "connecting…".to_string(),
            show_all_labels: prefs.show_all_labels,
            touch_select: prefs.touch_select,
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

    /// Build a sparse `pad_paging` delta by cloning the optimistic settings,
    /// applying `edit` to `pad_paging`, and diffing against the pre-edit copy.
    /// Returns `None` when settings are not loaded. Handles structural page ops
    /// (add/duplicate/delete/reorder) and name/color edits including clear-to-inherit.
    pub(crate) fn pad_paging_delta(
        &self,
        edit: impl FnOnce(&mut settings::PadPaging),
    ) -> Option<PartialSettings> {
        let base = self.settings.as_ref()?;
        let mut edited = (**base).clone();
        edit(&mut edited.pad_paging);
        Some(edited.diff_from(base))
    }

    /// Build + send a `pad_paging` structural/content delta. No-op if not loaded,
    /// and no-op when `edit` changes nothing: the driver rewrites the config file
    /// and pushes a full snapshot back for every persisted apply, so re-selecting
    /// the active page or committing an unchanged name must not send one.
    pub(crate) fn apply_pad_paging(
        &mut self,
        persist: bool,
        edit: impl FnOnce(&mut settings::PadPaging),
    ) {
        if let Some(delta) = self.pad_paging_delta(edit)
            && delta.pad_paging.is_some()
        {
            self.send_apply(delta, persist);
        }
    }

    /// Ask the driver to persist its current live settings to disk. The driver
    /// already holds the typed value from the live (`persist:false`) applies, so
    /// this flushes edits applied live but never committed via Enter — even if the
    /// selection has since changed. `seq` advances so the resulting snapshot is
    /// adopted by the same logic as a committed apply.
    pub(crate) fn persist_current(&mut self) {
        let Some(sender) = &self.sender else { return };
        self.seq += 1;
        let _ = sender.send(GuiToDriver::Persist { seq: self.seq });
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
        if self.confirm_delete_page.is_some() && self.settings.is_some() {
            return iced::widget::stack![
                base,
                crate::inspector::pages::view::delete_page_overlay(self)
            ]
            .into();
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
        // Run the persist-debounce timer only while a typed edit is awaiting its
        // quiet-window flush, so an idle GUI does zero periodic work.
        if self.persist_debounce > 0 {
            subs.push(
                iced::time::every(PERSIST_DEBOUNCE_INTERVAL).map(|_| Message::PersistDebounce),
            );
        }
        Subscription::batch(subs)
    }
}

#[cfg(test)]
mod pad_paging_delta_tests {
    use super::*;

    #[test]
    fn pad_paging_delta_is_none_without_settings() {
        let state = State::default();
        assert!(state.settings.is_none());
        assert!(state.pad_paging_delta(|_| {}).is_none());
    }

    #[test]
    fn apply_pad_paging_is_noop_without_settings() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut state = State {
            sender: Some(tx),
            ..State::default()
        };
        let seq_before = state.seq;

        state.apply_pad_paging(true, |pp| pp.enabled = true);

        assert_eq!(
            state.seq, seq_before,
            "no apply is sent when settings aren't loaded yet"
        );
        assert!(
            rx.try_recv().is_err(),
            "no frame reaches the driver when settings aren't loaded yet"
        );
    }

    #[test]
    fn apply_pad_paging_is_noop_when_the_edit_changes_nothing() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut state = State {
            sender: Some(tx),
            settings: Some(Arc::new(Settings::default())),
            ..State::default()
        };
        let seq_before = state.seq;

        // Re-selecting the already-active page: a persisted apply here would
        // rewrite the config file and push a full snapshot back for no change.
        state.apply_pad_paging(true, |pp| pp.active = 0);

        assert_eq!(state.seq, seq_before, "an empty delta bumps no seq");
        assert!(
            rx.try_recv().is_err(),
            "an empty delta reaches the driver as no frame at all"
        );
    }

    #[test]
    fn pad_paging_delta_is_confined_to_the_pad_paging_section() {
        let state = State {
            settings: Some(Arc::new(Settings::default())),
            ..State::default()
        };

        let delta = state
            .pad_paging_delta(|pp| pp.enabled = true)
            .expect("settings are loaded");

        assert!(delta.pad_paging.is_some(), "the edited section is present");
        assert!(delta.global.is_none());
        assert!(delta.hardware.is_none());
        assert!(delta.bridge.is_none());
        assert!(delta.driver.is_none());
        assert!(delta.pads.is_none());
        assert!(delta.buttons.is_none());
        assert!(delta.encoder.is_none());
        assert!(delta.slider.is_none());
    }
}
