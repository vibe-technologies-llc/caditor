use caditor_file::{Retention, Settings};
use caditor_render::DEFAULT_FIELD_OF_VIEW_DEGREES;

use crate::{
    appearance::TOOLTIP_DELAY,
    units::{Decimals, TrailingZeros},
    view_aids::EdgeWeight,
    view_cube::{Corner, CubeSize},
};

pub trait Choice: Copy + PartialEq + Default + 'static {
    const ALL: &'static [Self];

    fn key(self) -> &'static str;

    fn label(self) -> &'static str;

    fn description(self) -> &'static str;

    fn from_key(key: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|choice| choice.key() == key)
    }

    fn read(raw: &Settings, key: &str) -> Self {
        raw.text(key).and_then(Self::from_key).unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ZoomToward {
    #[default]
    Pointer,
    Centre,
}

impl Choice for ZoomToward {
    const ALL: &'static [Self] = &[Self::Pointer, Self::Centre];

    fn key(self) -> &'static str {
        match self {
            Self::Pointer => "pointer",
            Self::Centre => "centre",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Pointer => "The pointer",
            Self::Centre => "The view's centre",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Pointer => "Zooming keeps the point under the pointer where it is",
            Self::Centre => {
                "Zooming keeps the middle of the view where it is, wherever the pointer is"
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OrbitAround {
    #[default]
    Pointer,
    Centre,
}

impl Choice for OrbitAround {
    const ALL: &'static [Self] = &[Self::Pointer, Self::Centre];

    fn key(self) -> &'static str {
        match self {
            Self::Pointer => "pointer",
            Self::Centre => "centre",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Pointer => "The point under the pointer",
            Self::Centre => "The middle of the view",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Pointer => {
                "Orbiting turns about the model where the drag starts, or the view's centre over \
                 empty space"
            }
            Self::Centre => "Orbiting always turns about the middle of the view",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Transitions {
    Off,
    Quick,
    #[default]
    Standard,
    Slow,
}

impl Transitions {
    pub fn scale(self) -> f64 {
        match self {
            Self::Off => 0.0,
            Self::Quick => 0.5,
            Self::Standard => 1.0,
            Self::Slow => 2.0,
        }
    }
}

impl Choice for Transitions {
    const ALL: &'static [Self] = &[Self::Off, Self::Quick, Self::Standard, Self::Slow];

    fn key(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Quick => "quick",
            Self::Standard => "standard",
            Self::Slow => "slow",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Off => "No motion",
            Self::Quick => "Quick",
            Self::Standard => "Standard",
            Self::Slow => "Slow",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Off => {
                "Standard views, fitting and looking at a sketch jump straight to the new view, \
                 with no movement on screen"
            }
            Self::Quick => "View changes glide in half the usual time",
            Self::Standard => "View changes glide over about a third of a second",
            Self::Slow => "View changes glide over twice the usual time, easier to follow",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FieldOfView {
    Narrow,
    #[default]
    Standard,
    Wide,
}

impl FieldOfView {
    pub fn degrees(self) -> f64 {
        match self {
            Self::Narrow => 20.0,
            Self::Standard => DEFAULT_FIELD_OF_VIEW_DEGREES,
            Self::Wide => 45.0,
        }
    }
}

impl Choice for FieldOfView {
    const ALL: &'static [Self] = &[Self::Narrow, Self::Standard, Self::Wide];

    fn key(self) -> &'static str {
        match self {
            Self::Narrow => "narrow",
            Self::Standard => "standard",
            Self::Wide => "wide",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Narrow => "Narrow (20°)",
            Self::Standard => "Standard (30°)",
            Self::Wide => "Wide (45°)",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Narrow => "Less perspective: far parts shrink less, like a long lens",
            Self::Standard => "caditor's usual perspective",
            Self::Wide => "More perspective: near parts loom larger, like a wide-angle lens",
        }
    }
}

impl Choice for EdgeWeight {
    const ALL: &'static [Self] = &[Self::Thin, Self::Standard, Self::Thick];

    fn key(self) -> &'static str {
        match self {
            Self::Thin => "thin",
            Self::Standard => "standard",
            Self::Thick => "thick",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Thin => "Thin",
            Self::Standard => "Standard",
            Self::Thick => "Thick",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Thin => "Body edges and outlines drawn thinner, for dense models",
            Self::Standard => "Body edges and outlines at their usual width",
            Self::Thick => "Body edges and outlines drawn wider, easier to see",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TooltipDelay {
    AtOnce,
    #[default]
    Standard,
    Later,
}

impl TooltipDelay {
    pub fn seconds(self) -> f32 {
        match self {
            Self::AtOnce => 0.0,
            Self::Standard => TOOLTIP_DELAY,
            Self::Later => 1.0,
        }
    }
}

impl Choice for TooltipDelay {
    const ALL: &'static [Self] = &[Self::AtOnce, Self::Standard, Self::Later];

    fn key(self) -> &'static str {
        match self {
            Self::AtOnce => "at_once",
            Self::Standard => "standard",
            Self::Later => "later",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::AtOnce => "At once",
            Self::Standard => "Standard",
            Self::Later => "After a second",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::AtOnce => "Hover help shows as soon as the pointer rests on something",
            Self::Standard => "Hover help shows after about a third of a second",
            Self::Later => "Hover help waits a second, so it appears less often by accident",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Startup {
    #[default]
    EmptyModel,
    LastModel,
}

impl Choice for Startup {
    const ALL: &'static [Self] = &[Self::EmptyModel, Self::LastModel];

    fn key(self) -> &'static str {
        match self {
            Self::EmptyModel => "empty",
            Self::LastModel => "last",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::EmptyModel => "A new model",
            Self::LastModel => "The last model",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::EmptyModel => "caditor starts with a new, empty model",
            Self::LastModel => {
                "caditor opens the model at the top of Open recent, unless a file was named on \
                 the command line or unsaved work is offered for recovery"
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RecentListed {
    Three,
    Five,
    #[default]
    Ten,
}

impl RecentListed {
    pub fn count(self) -> usize {
        match self {
            Self::Three => 3,
            Self::Five => 5,
            Self::Ten => 10,
        }
    }
}

impl Choice for RecentListed {
    const ALL: &'static [Self] = &[Self::Three, Self::Five, Self::Ten];

    fn key(self) -> &'static str {
        match self {
            Self::Three => "3",
            Self::Five => "5",
            Self::Ten => "10",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Three => "3",
            Self::Five => "5",
            Self::Ten => "10",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Three => "Open recent and the welcome list the 3 newest models",
            Self::Five => "Open recent and the welcome list the 5 newest models",
            Self::Ten => "Open recent and the welcome list the 10 newest models",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DoubleClick {
    #[default]
    Edit,
    Rename,
}

impl Choice for DoubleClick {
    const ALL: &'static [Self] = &[Self::Edit, Self::Rename];

    fn key(self) -> &'static str {
        match self {
            Self::Edit => "edit",
            Self::Rename => "rename",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Edit => "Edits it",
            Self::Rename => "Renames it",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Edit => {
                "Double-clicking a feature opens it for editing, as Enter does; F2 renames"
            }
            Self::Rename => {
                "Double-clicking a feature's name starts renaming it; Enter still opens it for \
                 editing"
            }
        }
    }
}

impl Choice for Retention {
    const ALL: &'static [Self] = &Retention::ALL;

    fn key(self) -> &'static str {
        match self {
            Self::More => "more",
            Self::Standard => "standard",
            Self::Fewer => "fewer",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::More => "More",
            Self::Standard => "Standard",
            Self::Fewer => "Fewer",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::More => {
                "The last 25 saves, then one every quarter of an hour for two days, every four \
                 hours for three months, daily for two years and weekly after that"
            }
            Self::Standard => {
                "The last 10 saves, then one an hour for a day, a day for a month, a week for a \
                 year and a month after that"
            }
            Self::Fewer => {
                "The last 5 saves, then one every six hours for a day, a week for a month, a \
                 month for a year and a year after that, for smaller files"
            }
        }
    }
}

impl Choice for CubeSize {
    const ALL: &'static [Self] = &[Self::Small, Self::Standard, Self::Large];

    fn key(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Standard => "standard",
            Self::Large => "large",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Small => "Small",
            Self::Standard => "Standard",
            Self::Large => "Large",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Small => "A smaller view cube, leaving more of the view to the model",
            Self::Standard => "The view cube at its usual size",
            Self::Large => "A larger view cube, with bigger faces to click",
        }
    }
}

impl Choice for Corner {
    const ALL: &'static [Self] = &Corner::ALL;

    fn key(self) -> &'static str {
        match self {
            Self::TopLeft => "top_left",
            Self::TopRight => "top_right",
            Self::BottomLeft => "bottom_left",
            Self::BottomRight => "bottom_right",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::TopLeft => "Top left",
            Self::TopRight => "Top right",
            Self::BottomLeft => "Bottom left",
            Self::BottomRight => "Bottom right",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::TopLeft => "The view cube, Fit all and the Isometric view in the top left corner",
            Self::TopRight => {
                "The view cube, Fit all and the Isometric view in the top right corner"
            }
            Self::BottomLeft => {
                "The view cube, Fit all and the Isometric view in the bottom left corner; the \
                 axes move beside them"
            }
            Self::BottomRight => {
                "The view cube, Fit all and the Isometric view in the bottom right corner; the \
                 mouse hints move beside them"
            }
        }
    }
}

impl Choice for Decimals {
    const ALL: &'static [Self] = &[
        Self::Automatic,
        Self::Fixed(0),
        Self::Fixed(1),
        Self::Fixed(2),
        Self::Fixed(3),
        Self::Fixed(4),
        Self::Fixed(5),
        Self::Fixed(6),
    ];

    fn key(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::Fixed(0) => "0",
            Self::Fixed(1) => "1",
            Self::Fixed(2) => "2",
            Self::Fixed(3) => "3",
            Self::Fixed(4) => "4",
            Self::Fixed(5) => "5",
            Self::Fixed(_) => "6",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Automatic => "Automatic",
            Self::Fixed(0) => "None",
            Self::Fixed(1) => "1 place",
            Self::Fixed(2) => "2 places",
            Self::Fixed(3) => "3 places",
            Self::Fixed(4) => "4 places",
            Self::Fixed(5) => "5 places",
            Self::Fixed(_) => "6 places",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Automatic => {
                "Each readout shows the places it needs: a micrometre in measurements, a \
                 hundredth of a millimetre by the pointer, up to six places for computed values"
            }
            Self::Fixed(0) => "Every readout rounds to a whole number",
            Self::Fixed(1) => "Every readout shows one decimal place",
            Self::Fixed(2) => "Every readout shows two decimal places",
            Self::Fixed(3) => "Every readout shows three decimal places",
            Self::Fixed(4) => "Every readout shows four decimal places",
            Self::Fixed(5) => "Every readout shows five decimal places",
            Self::Fixed(_) => "Every readout shows six decimal places",
        }
    }
}

impl Choice for TrailingZeros {
    const ALL: &'static [Self] = &[Self::Shown, Self::Hidden];

    fn key(self) -> &'static str {
        match self {
            Self::Shown => "shown",
            Self::Hidden => "hidden",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Shown => "Shown",
            Self::Hidden => "Hidden",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Shown => "Readouts keep their zeros, so 12.500 mm lines up with 12.345 mm",
            Self::Hidden => "Readouts drop zeros after the point, so 12.500 mm reads 12.5 mm",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys_are_distinct_and_read_back<T: Choice + std::fmt::Debug>() {
        for (index, choice) in T::ALL.iter().enumerate() {
            assert_eq!(T::from_key(choice.key()), Some(*choice));
            assert!(
                !T::ALL[index + 1..]
                    .iter()
                    .any(|other| other.key() == choice.key())
            );
            assert!(!choice.label().is_empty() && !choice.description().is_empty());
        }
        let mut raw = Settings::default();
        raw.set_text("test.choice", "from a newer caditor");
        assert_eq!(T::read(&raw, "test.choice"), T::default());
        assert_eq!(T::read(&Settings::default(), "test.choice"), T::default());
    }

    #[test]
    fn every_choice_reads_back_its_own_key_and_falls_back_to_its_default() {
        keys_are_distinct_and_read_back::<ZoomToward>();
        keys_are_distinct_and_read_back::<OrbitAround>();
        keys_are_distinct_and_read_back::<Transitions>();
        keys_are_distinct_and_read_back::<FieldOfView>();
        keys_are_distinct_and_read_back::<EdgeWeight>();
        keys_are_distinct_and_read_back::<TooltipDelay>();
        keys_are_distinct_and_read_back::<Startup>();
        keys_are_distinct_and_read_back::<RecentListed>();
        keys_are_distinct_and_read_back::<DoubleClick>();
        keys_are_distinct_and_read_back::<Decimals>();
        keys_are_distinct_and_read_back::<CubeSize>();
        keys_are_distinct_and_read_back::<Corner>();
        keys_are_distinct_and_read_back::<Retention>();
        keys_are_distinct_and_read_back::<TrailingZeros>();
    }

    #[test]
    fn the_defaults_are_today_s_behaviour() {
        assert_eq!(Transitions::default().scale(), 1.0);
        assert_eq!(
            FieldOfView::default().degrees(),
            DEFAULT_FIELD_OF_VIEW_DEGREES
        );
        assert_eq!(EdgeWeight::default().factor(), 1.0);
        assert_eq!(RecentListed::default().count(), caditor_file::RECENT_LIMIT);
        assert_eq!(TooltipDelay::default().seconds(), TOOLTIP_DELAY);
    }
}
