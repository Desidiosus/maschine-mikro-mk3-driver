use maschine_library::controls::Buttons;
use protocol::ControlRef;
use resvg::usvg;
use settings::pads_by_index::config_key_to_internal;

/// The id'd device SVG, embedded so the binary is self-contained.
pub const DEVICE_SVG: &[u8] = include_bytes!("../../assets/maschine-mikro-mk3.svg");

/// SVG id → button, paired with the canonical `Buttons` variant so the diagram
/// labels stay tied to the device's button enum. Encoder push/touch have no
/// distinct diagram element.
const BUTTON_IDS: &[(&str, Buttons)] = &[
    ("Maschine button", Buttons::Maschine),
    ("Star", Buttons::Star),
    ("Browse", Buttons::Browse),
    ("Volume", Buttons::Volume),
    ("Swing", Buttons::Swing),
    ("Tempo button", Buttons::Tempo),
    ("Plug-in", Buttons::Plugin),
    ("Sampling", Buttons::Sampling),
    ("Left arrow", Buttons::Left),
    ("Right arrow", Buttons::Right),
    ("Pitch", Buttons::Pitch),
    ("Mod", Buttons::Mod),
    ("Perform", Buttons::Perform),
    ("Notes", Buttons::Notes),
    ("Group", Buttons::Group),
    ("Auto", Buttons::Auto),
    ("Lock", Buttons::Lock),
    ("Note repeat", Buttons::NoteRepeat),
    ("Restart", Buttons::Restart),
    ("Erase", Buttons::Erase),
    ("Tap", Buttons::Tap),
    ("Follow", Buttons::Follow),
    ("Play", Buttons::Play),
    ("Rec", Buttons::Rec),
    ("Stop", Buttons::Stop),
    ("Shift", Buttons::Shift),
    ("Fixed vel", Buttons::FixedVol),
    ("Pad mode", Buttons::PadMode),
    ("Keyboard", Buttons::Keyboard),
    ("Chords", Buttons::Chords),
    ("Step", Buttons::Step),
    ("Scene", Buttons::Scene),
    ("Pattern", Buttons::Pattern),
    ("Events", Buttons::Events),
    ("Variation", Buttons::Variation),
    ("Duplicate", Buttons::Duplicate),
    ("Select", Buttons::Select),
    ("Solo", Buttons::Solo),
    ("Mute", Buttons::Mute),
];

/// Human-readable label for a button index (from the SVG id table).
pub fn button_label(index: u8) -> &'static str {
    BUTTON_IDS
        .iter()
        .find(|(_, button)| *button as u8 == index)
        .map(|(label, _)| *label)
        .unwrap_or("Button")
}

/// Human-readable control name for the Assign header (not the MIDI label).
pub fn control_name(control: ControlRef) -> String {
    match control {
        ControlRef::Pad(i) => format!(
            "Pad {}",
            settings::pads_by_index::internal_to_config_key(i as usize)
        ),
        ControlRef::Button(i) => button_label(i).to_string(),
        ControlRef::Encoder => "Encoder".to_string(),
        ControlRef::Slider => "Touchstrip".to_string(),
    }
}

/// Axis-aligned rect in device (SVG) coordinate space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.x + self.w && py >= self.y && py <= self.y + self.h
    }

    fn intersects(&self, other: &Rect) -> bool {
        self.x < other.x + other.w
            && self.x + self.w > other.x
            && self.y < other.y + other.h
            && self.y + self.h > other.y
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Hotspot {
    pub rect: Rect,
    pub control: ControlRef,
}

/// Contain-fit the `dev_w`×`dev_h` device into a `bounds_w`×`bounds_h` box:
/// returns `(offset_x, offset_y, scale)` for a centered, aspect-preserving draw
/// (letterboxed). Picks the limiting dimension so the device is never clipped.
pub fn fit_contain(bounds_w: f32, bounds_h: f32, dev_w: f32, dev_h: f32) -> (f32, f32, f32) {
    let scale = (bounds_w / dev_w).min(bounds_h / dev_h).max(0.0);
    let draw_w = dev_w * scale;
    let draw_h = dev_h * scale;
    ((bounds_w - draw_w) / 2.0, (bounds_h - draw_h) / 2.0, scale)
}

/// Inset reserved around the device raster for its 1px border (1px stroke + 1px
/// breathing room on each side).
pub const DEVICE_INSET: f32 = 2.0;

/// The device→canvas transform `(origin_x, origin_y, scale)` for a `bounds` box,
/// accounting for the border inset. Hit-testing and drawing MUST both use this
/// so clicks land where the device and its labels are actually rendered.
pub fn device_transform(bounds_w: f32, bounds_h: f32, dev_w: f32, dev_h: f32) -> (f32, f32, f32) {
    let (fx, fy, scale) = fit_contain(
        bounds_w - 2.0 * DEVICE_INSET,
        bounds_h - 2.0 * DEVICE_INSET,
        dev_w,
        dev_h,
    );
    (fx + DEVICE_INSET, fy + DEVICE_INSET, scale)
}

/// Same `ControlRef` kind discriminant (Pad vs Button vs Encoder vs Slider) —
/// used so drag-select only groups controls of one type.
fn kind(control: &ControlRef) -> u8 {
    match control {
        ControlRef::Pad(_) => 0,
        ControlRef::Button(_) => 1,
        ControlRef::Encoder => 2,
        ControlRef::Slider => 3,
    }
}

pub struct Device {
    /// Native SVG size (1520 x 842).
    pub size: (f32, f32),
    pub hotspots: Vec<Hotspot>,
    /// Union of the 16 pad hotspot rects in device space: the pad-grid area the
    /// hardware frame and the page selector both anchor to. `None` if the SVG
    /// exposed no pad ids at all. Resolved once here because the hotspot table
    /// never changes after `load`, and the frame is re-derived on every canvas
    /// draw and every responsive layout pass.
    pad_grid: Option<Rect>,
}

impl Device {
    /// Parse the embedded SVG and build the hotspot table from its ids.
    pub fn load() -> Self {
        let tree = usvg::Tree::from_data(DEVICE_SVG, &usvg::Options::default())
            .expect("embedded device SVG must parse");
        let s = tree.size();
        let size = (s.width(), s.height());

        let mut hotspots = Vec::new();
        let mut push = |tree: &usvg::Tree, id: &str, control: ControlRef| {
            if let Some(node) = tree.node_by_id(id) {
                let b = node.abs_bounding_box();
                hotspots.push(Hotspot {
                    rect: Rect {
                        x: b.x(),
                        y: b.y(),
                        w: b.width(),
                        h: b.height(),
                    },
                    control,
                });
            }
        };

        for n in 1..=16usize {
            let id = format!("Pad {n}");
            push(&tree, &id, ControlRef::Pad(config_key_to_internal(n) as u8));
        }
        for (id, button) in BUTTON_IDS {
            push(&tree, id, ControlRef::Button(*button as u8));
        }
        push(&tree, "Encoder", ControlRef::Encoder);
        push(&tree, "Slider", ControlRef::Slider);

        let mut device = Self {
            size,
            hotspots,
            pad_grid: None,
        };
        device.pad_grid = device.pad_grid_union();
        device
    }

    /// Control whose hotspot contains a device-space point. Overlaps resolve by
    /// hotspot `Vec` order: pads → buttons → encoder → slider.
    pub fn hit_test(&self, x: f32, y: f32) -> Option<ControlRef> {
        self.hotspots
            .iter()
            .find(|h| h.rect.contains(x, y))
            .map(|h| h.control)
    }

    /// Hotspot rect for a control (for the Selection Frame + Touch-Select).
    pub fn rect_for(&self, control: ControlRef) -> Option<Rect> {
        self.hotspots
            .iter()
            .find(|h| h.control == control)
            .map(|h| h.rect)
    }

    /// The pad-grid area in device space (see the `pad_grid` field).
    pub fn pad_grid_rect(&self) -> Option<Rect> {
        self.pad_grid
    }

    fn pad_grid_union(&self) -> Option<Rect> {
        let mut pads = (0..16u8).filter_map(|i| self.rect_for(ControlRef::Pad(i)));
        let first = pads.next()?;
        Some(pads.fold(first, |acc, r| {
            let x = acc.x.min(r.x);
            let y = acc.y.min(r.y);
            let x2 = (acc.x + acc.w).max(r.x + r.w);
            let y2 = (acc.y + acc.h).max(r.y + r.h);
            Rect {
                x,
                y,
                w: x2 - x,
                h: y2 - y,
            }
        }))
    }

    /// Controls whose hotspots intersect `sel`, reduced to a single control kind.
    /// The kind is chosen by majority of intersecting hotspots. Encoder and
    /// Slider are eligible only when no Pad or Button is under the rect. Ties
    /// break by precedence Pad > Button > Encoder > Slider.
    pub fn controls_in_rect(&self, sel: Rect) -> Vec<ControlRef> {
        let hits: Vec<ControlRef> = self
            .hotspots
            .iter()
            .filter(|h| h.rect.intersects(&sel))
            .map(|h| h.control)
            .collect();
        if hits.is_empty() {
            return Vec::new();
        }
        let has_pad_or_button = hits
            .iter()
            .any(|c| matches!(c, ControlRef::Pad(_) | ControlRef::Button(_)));
        let eligible = |c: &ControlRef| {
            !has_pad_or_button || matches!(c, ControlRef::Pad(_) | ControlRef::Button(_))
        };

        let mut best_kind: Option<u8> = None;
        let mut best_count = 0usize;
        for k in [0u8, 1, 2, 3] {
            let count = hits.iter().filter(|c| eligible(c) && kind(c) == k).count();
            if count > best_count {
                best_count = count;
                best_kind = Some(k);
            }
        }
        match best_kind {
            Some(k) => hits.into_iter().filter(|c| kind(c) == k).collect(),
            None => Vec::new(),
        }
    }
}

#[cfg(test)]
mod fit_tests {
    use super::fit_contain;

    #[test]
    fn wide_box_letterboxes_horizontally_height_limited() {
        // Box wider than the 1520:842 device aspect → height-limited, centered x.
        let (ox, oy, scale) = fit_contain(4000.0, 842.0, 1520.0, 842.0);
        assert!((scale - 1.0).abs() < 1e-3);
        assert!(oy.abs() < 1e-3);
        assert!(ox > 0.0); // horizontal letterbox
    }

    #[test]
    fn tall_box_letterboxes_vertically_width_limited() {
        // Box taller/narrower than the device aspect → width-limited, centered y.
        let (ox, oy, scale) = fit_contain(1520.0, 2000.0, 1520.0, 842.0);
        assert!((scale - 1.0).abs() < 1e-3);
        assert!(ox.abs() < 1e-3);
        assert!(oy > 0.0); // vertical letterbox
        // Never clipped: drawn height <= bounds height.
        assert!(842.0 * scale <= 2000.0 + 1e-3);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_controls_resolve_to_hotspots() {
        let d = Device::load();
        // 16 pads + 39 buttons + encoder + slider.
        assert_eq!(d.hotspots.len(), 57, "every control id must resolve");
        assert_eq!(d.size, (1520.0, 842.0));
    }

    #[test]
    fn pad_1_maps_to_internal_12_and_sits_bottom_left() {
        let d = Device::load();
        // Physical pad 1 (bottom-left) = config_key_to_internal(1) = internal 12.
        let r = d.rect_for(ControlRef::Pad(12)).expect("pad 1 hotspot");
        // bottom-left of the pad grid (left side, lower half of the canvas).
        assert!(r.x < 1000.0, "pad 1 is on the left of the grid: {r:?}");
        assert!(r.y > 400.0, "pad 1 is in the lower half: {r:?}");
    }

    #[test]
    fn pad_grid_rect_is_the_tight_union_of_every_pad() {
        let device = Device::load();
        let grid = device.pad_grid_rect().expect("device SVG exposes pads");
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;
        for i in 0..16u8 {
            let r = device.rect_for(ControlRef::Pad(i)).unwrap();
            assert!(grid.x <= r.x && grid.y <= r.y);
            assert!(grid.x + grid.w >= r.x + r.w);
            assert!(grid.y + grid.h >= r.y + r.h);
            min_x = min_x.min(r.x);
            min_y = min_y.min(r.y);
            max_x = max_x.max(r.x + r.w);
            max_y = max_y.max(r.y + r.h);
        }
        // Tight, not merely containing: an oversized rect would drag the frame
        // and the selector anchored to it away from the pads. The origin is a
        // plain minimum so it must match exactly; the far edges are compared
        // with a sub-thousandth-of-a-device-unit tolerance because the union
        // stores a width, and recovering `x + w` back to the maximum edge is
        // not guaranteed to round-trip exactly in f32.
        assert_eq!((grid.x, grid.y), (min_x, min_y));
        assert!(
            (grid.x + grid.w - max_x).abs() < 1e-3,
            "right edge: {grid:?}"
        );
        assert!(
            (grid.y + grid.h - max_y).abs() < 1e-3,
            "bottom edge: {grid:?}"
        );
    }

    #[test]
    fn hit_test_returns_the_control_at_a_point() {
        let d = Device::load();
        let r = d.rect_for(ControlRef::Encoder).expect("encoder hotspot");
        let hit = d.hit_test(r.x + r.w / 2.0, r.y + r.h / 2.0);
        assert_eq!(hit, Some(ControlRef::Encoder));
    }

    #[test]
    fn drag_select_whole_device_picks_buttons_by_majority() {
        let d = Device::load();
        let sel = Rect {
            x: 0.0,
            y: 0.0,
            w: d.size.0,
            h: d.size.1,
        };
        let controls = d.controls_in_rect(sel);
        assert!(!controls.is_empty());
        assert!(
            controls.iter().all(|c| matches!(c, ControlRef::Button(_))),
            "39 buttons outnumber 16 pads, so the whole device selects buttons: {controls:?}"
        );
        let button_hotspots = d
            .hotspots
            .iter()
            .filter(|h| matches!(h.control, ControlRef::Button(_)))
            .count();
        assert_eq!(controls.len(), button_hotspots, "all buttons are selected");
    }

    #[test]
    fn drag_select_pad_grid_only_returns_pads() {
        let d = Device::load();
        let sel = Rect {
            x: 800.0,
            y: 120.0,
            w: 680.0,
            h: 680.0,
        };
        let controls = d.controls_in_rect(sel);
        assert!(!controls.is_empty());
        assert!(controls.iter().all(|c| matches!(c, ControlRef::Pad(_))));
    }

    #[test]
    fn drag_select_excludes_slider_and_encoder_when_pads_or_buttons_present() {
        let d = Device::load();
        let sel = Rect {
            x: 0.0,
            y: 0.0,
            w: d.size.0,
            h: d.size.1,
        };
        let controls = d.controls_in_rect(sel);
        assert!(
            !controls
                .iter()
                .any(|c| matches!(c, ControlRef::Encoder | ControlRef::Slider)),
            "encoder/slider are dropped when pads/buttons are under the rect: {controls:?}"
        );
    }

    #[test]
    fn drag_select_returns_slider_when_it_is_the_only_control() {
        let d = Device::load();
        let r = d.rect_for(ControlRef::Slider).expect("slider hotspot");
        let sel = Rect {
            x: r.x + 1.0,
            y: r.y + 1.0,
            w: (r.w - 2.0).max(0.0),
            h: (r.h - 2.0).max(0.0),
        };
        let controls = d.controls_in_rect(sel);
        assert_eq!(controls, vec![ControlRef::Slider]);
    }
}
