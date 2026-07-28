use serde::{Deserialize, Serialize};

use crate::defaults::default_pads;
use crate::pads_by_index::PadsByIndex;
use maschine_library::lights::PadColors;

/// Fewest pages a validated `PadPaging` may contain.
pub const MIN_PAGES: usize = 1;
/// Most pages a validated `PadPaging` may contain.
pub const MAX_PAGES: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PadPage {
    /// Stable identity, assigned once at creation and carried through renames,
    /// reorders and persistence. This — not the slot index, and not the name —
    /// is what says whether two page values are the same page. Left out of the
    /// serialized form while unassigned, so the generated reference config does
    /// not advertise an identity as if it were a tunable default.
    #[serde(default, skip_serializing_if = "is_unassigned_page_id")]
    pub id: PageId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<PadColors>,
    pub pads: PadsByIndex,
}

/// Identity of a page. Handed out by `PadPaging::next_page_id` as one past the
/// highest in use — a plain counter rather than a random id, so the settings
/// crate stays deterministic (no rng, reproducible tests) and a hand-edited
/// config reads `id = 3` instead of a uuid.
pub type PageId = u32;

/// The id a page has before anyone assigns one: a blank `default_page` template,
/// or a page loaded from a config written before ids existed. It is a perfectly
/// usable id — only pages that would *share* one are renumbered — so a
/// single-page config keeps it and round-trips unchanged.
pub const UNASSIGNED_PAGE_ID: PageId = 0;

fn is_unassigned_page_id(id: &PageId) -> bool {
    *id == UNASSIGNED_PAGE_ID
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PadPaging {
    pub enabled: bool,
    /// Index into `pages`. `merge_overrides` clamps a stale value (a shrunk page
    /// list from an earlier layer) into range and `validate` rejects anything
    /// still out of range, so a live `Settings` always has this pointing at a
    /// real page. Read it through `active_page`/`active_page_mut` rather than
    /// indexing `pages` directly, so an unvalidated value resolves the same way
    /// everywhere instead of silently dropping the edit that used it.
    pub active: usize,
    pub default_page_color: PadColors,
    pub pages: Vec<PadPage>,
}

impl PadPaging {
    pub fn page_color(&self, page: &PadPage) -> PadColors {
        page.color.unwrap_or(self.default_page_color)
    }

    /// Index of the active page, clamped into `0..pages.len()`. `pages` is never
    /// empty for a validated `Settings`, so this always resolves to a real page;
    /// the clamp is defensive against an out-of-range `active` that has not yet
    /// been validated (or self-healed by `merge_overrides`).
    fn active_index(&self) -> usize {
        self.active.min(self.pages.len().saturating_sub(1))
    }

    pub fn active_page(&self) -> &PadPage {
        &self.pages[self.active_index()]
    }

    pub fn active_page_mut(&mut self) -> &mut PadPage {
        let idx = self.active_index();
        &mut self.pages[idx]
    }

    /// Id for a page created now: one past the highest in use, so it collides
    /// neither with a page that exists nor with one a `-c` seed still holds.
    pub fn next_page_id(&self) -> PageId {
        self.pages
            .iter()
            .map(|page| page.id)
            .max()
            .unwrap_or(UNASSIGNED_PAGE_ID)
            .saturating_add(1)
    }

    /// A blank page stamped with a fresh id, ready to be named and pushed. Every
    /// caller that creates a page should go through here rather than
    /// `default_page`, which is the unstamped template.
    pub fn new_page(&self) -> PadPage {
        PadPage {
            id: self.next_page_id(),
            ..default_page()
        }
    }

    /// Renumber any page that shares an id with an earlier one. Called on every
    /// merge, so a config written before ids existed (where every page loads as
    /// `UNASSIGNED_PAGE_ID`) or hand-edited into duplicates self-heals instead of
    /// leaving two pages that claim to be the same page. Pages whose id is
    /// already unique are left alone, so a merge that changes nothing else
    /// changes nothing here either.
    pub(crate) fn ensure_unique_page_ids(&mut self) {
        let mut taken: Vec<PageId> = Vec::with_capacity(self.pages.len());
        let mut next = self.next_page_id();
        for page in self.pages.iter_mut() {
            if taken.contains(&page.id) {
                page.id = next;
                next += 1;
            }
            taken.push(page.id);
        }
    }

    /// Default letter names ("Pad Page A"…"Pad Page Z") that no page currently
    /// stores, in order. Handing names out from here is what keeps a generated
    /// name from colliding with one the user typed.
    fn unused_default_names(&self) -> impl Iterator<Item = String> + '_ {
        let taken: Vec<&str> = self
            .pages
            .iter()
            .filter_map(|p| p.name.as_deref())
            .collect();
        ('A'..='Z')
            .map(|letter| format!("Pad Page {letter}"))
            .filter(move |candidate| !taken.contains(&candidate.as_str()))
    }

    /// Name to give the next page created: the first default letter name not
    /// already stored on a page. Position plays no part, so a reorder never
    /// renames anything.
    pub fn next_page_name(&self) -> String {
        self.unused_default_names()
            .next()
            .unwrap_or_else(|| format!("Pad Page {}", self.pages.len() + 1))
    }

    /// The name to show for page `index`: its stored name, or — for a page that
    /// carries none (a config written before pages had names, or a rename field
    /// the user has cleared) — a default letter name that collides with neither
    /// a stored name nor another unnamed page's fallback. Every surface that
    /// displays a page resolves it here, so the same page never reads one way on
    /// the device screen and another in the GUI.
    pub fn display_name(&self, index: usize) -> String {
        if let Some(name) = self.pages.get(index).and_then(|p| p.name.clone()) {
            return name;
        }
        let rank = self.pages[..index.min(self.pages.len())]
            .iter()
            .filter(|p| p.name.is_none())
            .count();
        self.unused_default_names()
            .nth(rank)
            .unwrap_or_else(|| format!("Pad Page {}", index + 1))
    }
}

/// The blank page template: no id and no name, both of which the creating
/// caller stamps (`PadPaging::new_page` for the id, which is why that is the
/// entry point to prefer). Also the base a partial page patch is applied onto,
/// where the id arrives with the patch or is filled in by the merge.
pub fn default_page() -> PadPage {
    PadPage {
        id: UNASSIGNED_PAGE_ID,
        name: None,
        color: None,
        pads: default_pads(),
    }
}

pub fn default_pad_paging() -> PadPaging {
    PadPaging {
        enabled: false,
        active: 0,
        default_page_color: PadColors::White,
        pages: vec![default_page()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_pad_paging_has_one_disabled_page() {
        let p = default_pad_paging();
        assert!(!p.enabled);
        assert_eq!(p.active, 0);
        assert_eq!(p.pages.len(), 1);
        assert_eq!(p.pages[0].pads, default_pads());
    }

    #[test]
    fn page_color_falls_back_to_default_when_unset() {
        let mut p = default_pad_paging();
        p.default_page_color = PadColors::Cyan;
        assert_eq!(p.page_color(&p.pages[0]), PadColors::Cyan);

        let mut page = p.pages[0].clone();
        page.color = Some(PadColors::Red);
        assert_eq!(p.page_color(&page), PadColors::Red);
    }

    #[test]
    fn active_page_clamps_when_active_out_of_range() {
        let mut p = default_pad_paging();
        // One page, but active points past it (as a stale persisted value could).
        p.active = 5;
        assert_eq!(p.active_page().pads, default_pads());
        p.active_page_mut().name = Some("x".to_string());
        assert_eq!(p.pages[0].name.as_deref(), Some("x"));
    }

    #[test]
    fn a_new_page_gets_an_id_no_other_page_holds() {
        let mut p = default_pad_paging();
        let first = p.new_page();
        p.pages.push(first.clone());
        let second = p.new_page();

        assert_ne!(first.id, p.pages[0].id);
        assert_ne!(second.id, first.id);
    }

    #[test]
    fn ids_survive_a_reorder_while_slots_and_letters_do_not() {
        // The point of the id: after dragging a page, the same page is still the
        // same page even though its index — and the letter shown for its slot —
        // changed.
        let mut p = default_pad_paging();
        p.pages.push(p.new_page());
        let (first, second) = (p.pages[0].id, p.pages[1].id);

        p.pages.swap(0, 1);

        assert_eq!((p.pages[0].id, p.pages[1].id), (second, first));
    }

    #[test]
    fn sharing_an_id_is_repaired_and_a_unique_one_is_left_alone() {
        // A config from before ids existed loads every page as unassigned; a
        // hand-edited one can repeat an id outright. Either way no two pages may
        // end up claiming to be the same page.
        let mut p = default_pad_paging();
        p.pages.push(default_page());
        p.pages.push(default_page());
        p.pages[2].id = 7;

        p.ensure_unique_page_ids();

        let ids: Vec<PageId> = p.pages.iter().map(|page| page.id).collect();
        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), ids.len(), "ids must be unique: {ids:?}");
        assert_eq!(
            (ids[0], ids[2]),
            (UNASSIGNED_PAGE_ID, 7),
            "only the page that collided is renumbered"
        );
    }

    #[test]
    fn an_unnamed_page_never_displays_another_pages_stored_name() {
        // Clearing a rename field stores `None`, so an unnamed page sitting next
        // to a page literally named "Pad Page A" is one keystroke away. Two rows
        // reading the same name leaves the delete dialog unable to say which
        // page it is about to remove.
        let mut p = default_pad_paging();
        p.pages[0].name = None;
        p.pages.push(PadPage {
            name: Some("Pad Page A".to_string()),
            ..default_page()
        });

        assert_eq!(p.display_name(0), "Pad Page B");
        assert_eq!(p.display_name(1), "Pad Page A");
    }

    #[test]
    fn several_unnamed_pages_get_distinct_display_names() {
        let mut p = default_pad_paging();
        p.pages[0].name = None;
        p.pages.push(default_page());

        assert_eq!(p.display_name(0), "Pad Page A");
        assert_eq!(p.display_name(1), "Pad Page B");
    }

    #[test]
    fn next_page_name_skips_names_already_in_use() {
        let mut p = default_pad_paging();
        // Spelled out rather than relying on the schema's starting page: what is
        // under test is that a taken name is skipped, not which names the
        // defaults happen to carry.
        p.pages[0].name = Some("Pad Page A".to_string());
        p.pages.push(PadPage {
            name: Some("Pad Page C".to_string()),
            ..default_page()
        });
        assert_eq!(p.next_page_name(), "Pad Page B");
    }

    #[test]
    fn unknown_page_key_is_rejected() {
        // Serialize a real page (16 pads) so the only difference is the stray key —
        // the error must come from deny_unknown_fields, not from incomplete pads.
        let good = toml::to_string(&default_page()).unwrap();
        assert!(
            toml::from_str::<PadPage>(&good).is_ok(),
            "a valid page round-trips"
        );

        let bad = format!("typo = true\n{good}");
        assert!(
            toml::from_str::<PadPage>(&bad).is_err(),
            "an unknown page field must be rejected, matching the rest of the schema"
        );
    }
}
