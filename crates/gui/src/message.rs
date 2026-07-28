//! The top-level `Message` type for the GUI application.

use std::sync::mpsc::Sender;

use protocol::{DriverToGui, GuiToDriver};
use settings::PadVelocityCurve;

use crate::inspector::assign::forms::{
    AssignTab, CcType, EncoderModeKind, LedTab, PadHitType, PadPressType, SliderTouchKind,
};
use crate::inspector::assign::mapping::PadLedColorSlot;
use crate::inspector::assign::numeric::EditField;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectorTab {
    Assign,
    Pages,
}

#[derive(Debug, Clone)]
pub enum Message {
    /// Connection established; carries the channel to send requests to the driver.
    Ready(Sender<GuiToDriver>),
    /// A frame arrived from the driver.
    Frame(DriverToGui),
    Disconnected,
    Error(String),
    /// Periodic redraw tick (fades the MIDI activity LEDs).
    Tick,
    SelectControl(protocol::ControlRef),
    SelectControls(Vec<protocol::ControlRef>),
    ToggleTouchSelect(bool),
    /// Slider drags apply live to the device (`persist:false`) and update the label.
    PreviewPadSensitivity(u8),
    PreviewDisplayContrast(u8),
    /// Slider release persists the current value via `Apply { persist: true }`.
    SetPadSensitivity,
    SetDisplayContrast,
    SetVelocityCurve(PadVelocityCurve),
    PreviewLedBrightness(u8),
    SetLedBrightness,
    /// Toggle the Preferences overlay.
    TogglePrefs,
    // Encoder mode + wrap (pick_list / checkbox).
    SetEncoderModeKind(EncoderModeKind),
    SetEncoderWrap(bool),
    // Slider touch kind + LED (pick_list / checkbox).
    SetSliderTouchKind(SliderTouchKind),
    SetSliderLedMode(settings::SliderLedMode),
    SetSliderLedColor(settings::PadColors),
    SetSliderLedStylized(bool),
    /// Persisted driver toggles from the Preferences overlay.
    ToggleSoftOff(bool),
    ToggleSelfTestAtLaunch(bool),
    // --- redesign: generic numeric + tabs + labels ---
    NumericInput(EditField, String),
    NumericStep(EditField, i8),
    NumericCommit(EditField),
    /// Debounce tick: typed edits apply live (`persist:false`); this fires after
    /// a quiet window to persist them, so a value is saved even without pressing
    /// Enter (iced gives no focus-lost callback to commit on).
    PersistDebounce,
    /// Pad pressure enable (Disabled vs Poly).
    SetPadPressureKind(bool),
    SetAssignTab(AssignTab),
    ToggleShowAllLabels(bool),
    /// Ctrl+click: toggle one control's membership in the current selection.
    /// Ignored when the control is a different kind than the current selection.
    ToggleControl(protocol::ControlRef),
    // Action Type selection (per sub-action slot).
    SetPadHitType(PadHitType),
    SetPadPressType(PadPressType),
    SetButtonType(CcType),
    SetEncoderTurnType(CcType),
    SetEncoderPushType(CcType),
    SetEncoderTouchType(CcType),
    SetSliderPositionType(CcType),
    // --- per-pad LED color ---
    SetPadLedSource(settings::PadLedSource),
    SetPadLedMode(LedTab, settings::PadLedMode),
    /// Set the color in one slot (Single / Dual-on / Dual-off) of `tab`'s mode.
    SetPadLedColor(LedTab, PadLedColorSlot, settings::PadColors),
    SetInspectorTab(InspectorTab),
    SetPagingEnabled(bool),
    SelectPage(usize),
    SetDefaultPageColor(settings::PadColors),
    AddPage,
    DuplicatePage(usize),
    /// The row-actions Delete button pressed for a page index: opens the
    /// confirmation dialog rather than deleting immediately.
    RequestDeletePage(usize),
    /// The confirmation dialog's Delete button: deletes the page named by
    /// `State::confirm_delete_page` and closes the dialog.
    ConfirmDeletePage,
    /// The confirmation dialog's Cancel button (or a scrim click): closes the
    /// dialog without deleting anything.
    CancelDeletePage,
    SetPageName(usize, String),
    /// Enter after typing a page name: trims the in-progress text and persists
    /// it. Typing itself (`SetPageName`) stores the raw text and applies live
    /// without trimming, since trimming every keystroke would swallow spaces
    /// before a trailing word is typed. iced's `text_input` reports no focus
    /// loss, so `update` also runs this commit from every path that closes the
    /// rename field.
    CommitPageName(usize),
    /// The row's pencil button pressed for a page index: shows that row's
    /// `text_input` in place of its plain-text name and focuses it. Only one
    /// row edits at a time.
    BeginRenamePage(usize),
    /// A press started on a page row (row-level `mouse_area`, not the styled
    /// `button` it wraps — the button would otherwise swallow the press
    /// before the `mouse_area` ever saw it). Carries the row index.
    PageDragStart(usize),
    /// The pointer entered a page row: tracks the hover highlight always, and
    /// the drag target when a row drag is in progress.
    PageRowEntered(usize),
    /// The pointer left a page row; clears the hover highlight if it still
    /// points at that row.
    PageRowExited(usize),
    /// The pointer was released anywhere over the page list. Commits a
    /// reorder when the drag crossed into a different row, or selects the
    /// origin row when it didn't (a plain click, or a drag that returned to
    /// where it started).
    PageDragDrop,
    /// Abandons an in-progress row drag (e.g. the pointer left the page list
    /// without a release ever landing on it) so `page_drag` can't get stuck.
    PageDragCancel,
    /// No-op; swallows clicks inside the Preferences panel so they don't reach
    /// the modal backdrop and close it.
    Ignore,
}
