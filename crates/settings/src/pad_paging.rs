use serde::{Deserialize, Serialize};

use crate::defaults::default_pads;
use crate::pads_by_index::PadsByIndex;
use maschine_library::lights::PadColors;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PadPage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<PadColors>,
    pub pads: PadsByIndex,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PadPaging {
    pub enabled: bool,
    pub active: usize,
    pub default_page_color: PadColors,
    pub pages: Vec<PadPage>,
}

impl PadPaging {
    pub fn page_color(&self, page: &PadPage) -> PadColors {
        page.color.unwrap_or(self.default_page_color)
    }

    pub fn active_page(&self) -> &PadPage {
        &self.pages[self.active]
    }

    pub fn active_page_mut(&mut self) -> &mut PadPage {
        let active = self.active;
        &mut self.pages[active]
    }
}

pub fn default_pad_paging() -> PadPaging {
    PadPaging {
        enabled: false,
        active: 0,
        default_page_color: PadColors::White,
        pages: vec![PadPage {
            name: None,
            color: None,
            pads: default_pads(),
        }],
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
}
