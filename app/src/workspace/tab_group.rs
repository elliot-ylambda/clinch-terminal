//! Tab group data model. Gated at runtime by `FeatureFlag::GroupedTabs`.

use std::fmt;
use std::str::FromStr;

use pathfinder_color::ColorU;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use warp_core::ui::theme::{AnsiColorIdentifier, AnsiColors};
use warpui::elements::DraggableState;

use crate::tab::SelectedTabColor;
use crate::ui_components::CLINCH_SECTION_GREEN;

/// Colors available to sidebar sections. The six standard choices mirror the tab-color picker;
/// Clinch Green is a branded section-only option.
pub(crate) const SECTION_COLOR_OPTIONS: [SectionColor; 7] = [
    SectionColor::Red,
    SectionColor::Green,
    SectionColor::ClinchGreen,
    SectionColor::Yellow,
    SectionColor::Blue,
    SectionColor::Magenta,
    SectionColor::Cyan,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionColor {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    ClinchGreen,
}

impl SectionColor {
    pub(crate) fn to_color(self, ansi_colors: &AnsiColors) -> ColorU {
        match self {
            Self::ClinchGreen => CLINCH_SECTION_GREEN,
            _ => self
                .ansi_color_identifier()
                .expect("non-branded section colors are ANSI colors")
                .to_ansi_color(ansi_colors)
                .into(),
        }
    }

    fn ansi_color_identifier(self) -> Option<AnsiColorIdentifier> {
        Some(match self {
            Self::Black => AnsiColorIdentifier::Black,
            Self::Red => AnsiColorIdentifier::Red,
            Self::Green => AnsiColorIdentifier::Green,
            Self::Yellow => AnsiColorIdentifier::Yellow,
            Self::Blue => AnsiColorIdentifier::Blue,
            Self::Magenta => AnsiColorIdentifier::Magenta,
            Self::Cyan => AnsiColorIdentifier::Cyan,
            Self::White => AnsiColorIdentifier::White,
            Self::ClinchGreen => return None,
        })
    }
}

impl From<AnsiColorIdentifier> for SectionColor {
    fn from(value: AnsiColorIdentifier) -> Self {
        match value {
            AnsiColorIdentifier::Black => Self::Black,
            AnsiColorIdentifier::Red => Self::Red,
            AnsiColorIdentifier::Green => Self::Green,
            AnsiColorIdentifier::Yellow => Self::Yellow,
            AnsiColorIdentifier::Blue => Self::Blue,
            AnsiColorIdentifier::Magenta => Self::Magenta,
            AnsiColorIdentifier::Cyan => Self::Cyan,
            AnsiColorIdentifier::White => Self::White,
        }
    }
}

impl fmt::Display for SectionColor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Black => "black",
            Self::Red => "red",
            Self::Green => "green",
            Self::Yellow => "yellow",
            Self::Blue => "blue",
            Self::Magenta => "magenta",
            Self::Cyan => "cyan",
            Self::White => "white",
            Self::ClinchGreen => "clinch-green",
        };
        formatter.write_str(name)
    }
}

impl FromStr for SectionColor {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().replace('_', "-").as_str() {
            "black" => Ok(Self::Black),
            "red" => Ok(Self::Red),
            "green" => Ok(Self::Green),
            "yellow" => Ok(Self::Yellow),
            "blue" => Ok(Self::Blue),
            "magenta" => Ok(Self::Magenta),
            "cyan" => Ok(Self::Cyan),
            "white" => Ok(Self::White),
            "clinch-green" | "clinchgreen" => Ok(Self::ClinchGreen),
            _ => Err(()),
        }
    }
}

/// A persisted section-color override. `Unset` uses a built-in section's default, while
/// `Cleared` explicitly removes that default tint.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectedSectionColor {
    #[default]
    Unset,
    Cleared,
    Color(SectionColor),
}

impl SelectedSectionColor {
    pub(crate) fn resolve(self, default: Option<SectionColor>) -> Option<SectionColor> {
        match self {
            Self::Color(color) => Some(color),
            Self::Cleared => None,
            Self::Unset => default,
        }
    }
}

impl From<SelectedTabColor> for SelectedSectionColor {
    fn from(value: SelectedTabColor) -> Self {
        match value {
            SelectedTabColor::Unset => Self::Unset,
            SelectedTabColor::Cleared => Self::Cleared,
            SelectedTabColor::Color(color) => Self::Color(color.into()),
        }
    }
}

/// Stable identity for a tab group.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TabGroupId(pub Uuid);

impl TabGroupId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for TabGroupId {
    fn default() -> Self {
        Self::new()
    }
}

/// A named group of tabs in the vertical tabs panel.
/// Member tabs reference their group via `TabData::group_id`.
#[derive(Clone)]
pub struct TabGroup {
    pub id: TabGroupId,
    pub name: Option<String>,
    pub color: SelectedSectionColor,
    pub collapsed: bool,
    pub draggable_state: DraggableState,
    /// True when this whole group is pinned to the front of the tab list.
    pub pinned: bool,
}

impl TabGroup {
    /// Creates a new, untitled, expanded tab group with a fresh id.
    pub fn new() -> Self {
        Self {
            id: TabGroupId::new(),
            name: None,
            color: SelectedSectionColor::default(),
            collapsed: false,
            draggable_state: Default::default(),
            pinned: false,
        }
    }
}

impl Default for TabGroup {
    fn default() -> Self {
        Self::new()
    }
}
