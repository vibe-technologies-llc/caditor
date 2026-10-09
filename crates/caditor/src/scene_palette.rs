use caditor_render::Color;

use crate::{canvas, scene::opaque, selection::Axis};

const fn translucent(color: egui::Color32, alpha: u8) -> Color {
    Color::from_rgba8(color.r(), color.g(), color.b(), alpha)
}

const CHOSEN_REGION_ALPHA: u8 = 90;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Contrast {
    #[default]
    Standard,
    High,
}

impl Contrast {
    pub fn of(high_contrast: bool) -> Self {
        if high_contrast {
            Self::High
        } else {
            Self::Standard
        }
    }

    pub fn palette(self) -> &'static ScenePalette {
        match self {
            Self::Standard => &STANDARD,
            Self::High => &HIGH_CONTRAST,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SketchState {
    UnderConstrained,
    FullyConstrained,
    Conflicting,
    Redundant,
    Failed,
    Projected,
    Background,
}

impl SketchState {
    #[cfg(test)]
    pub const ALL: [Self; 7] = [
        Self::UnderConstrained,
        Self::FullyConstrained,
        Self::Conflicting,
        Self::Redundant,
        Self::Failed,
        Self::Projected,
        Self::Background,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weight {
    Regular,
    Heavy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointFill {
    Solid,
    Hollow,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    pub curve: Color,
    pub point: Color,
    pub weight: Weight,
    pub fill: PointFill,
}

impl Look {
    const fn plain(curve: Color, point: Color) -> Self {
        Self {
            curve,
            point,
            weight: Weight::Regular,
            fill: PointFill::Solid,
        }
    }

    const fn formed(curve: Color, weight: Weight, fill: PointFill) -> Self {
        Self {
            curve,
            point: curve,
            weight,
            fill,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Highlights {
    pub hovered: Color,
    pub selected: Color,
    pub hovered_selected: Color,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bands {
    pub drafted: Color,
    pub too_little_draft: Color,
    pub undercut: Color,
    pub too_tight: Color,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScenePalette {
    pub grid: Color,
    pub origin: Color,
    pub plane_fill: Color,
    pub plane_edge: Color,
    pub axes: [Color; 3],
    pub sketch_horizontal_axis: Color,
    pub sketch_vertical_axis: Color,
    pub lines: Highlights,
    pub faces: Highlights,
    pub under_constrained: Look,
    pub fully_constrained: Look,
    pub conflicting: Look,
    pub redundant: Look,
    pub failed: Look,
    pub projected: Look,
    pub background: Look,
    pub hole: Color,
    pub preview_curve: Color,
    pub preview_point: Color,
    pub trimmed_curve: Color,
    pub cut_preview: Color,
    pub cut_preview_edge: Color,
    pub failed_body: Color,
    pub outdated_body: Color,
    pub background_body: Color,
    pub body_edge: Color,
    pub background_body_edge: Color,
    pub drawing_face: Color,
    pub chosen_region: Color,
    pub open_region: Color,
    pub closed_region: Color,
    pub revolve_axis: Color,
    pub followed_edge: Color,
    pub centre_of_mass: Color,
    pub thread: Color,
    pub bands: Bands,
    pub datum_edge: Color,
    pub datum_fill: Color,
    pub failed_datum_edge: Color,
    pub failed_datum_fill: Color,
    pub curve_width: f32,
    pub heavy_curve_width: f32,
    pub body_edge_width: f32,
    pub point_diameter: f32,
    pub hole_diameter: f32,
    pub selection_widening: f32,
    pub troubled_edges_dashed: bool,
}

impl ScenePalette {
    pub fn look(&self, state: SketchState) -> Look {
        match state {
            SketchState::UnderConstrained => self.under_constrained,
            SketchState::FullyConstrained => self.fully_constrained,
            SketchState::Conflicting => self.conflicting,
            SketchState::Redundant => self.redundant,
            SketchState::Failed => self.failed,
            SketchState::Projected => self.projected,
            SketchState::Background => self.background,
        }
    }

    pub fn curve_width(&self, weight: Weight) -> f32 {
        match weight {
            Weight::Regular => self.curve_width,
            Weight::Heavy => self.heavy_curve_width,
        }
    }

    pub fn axis(&self, axis: Axis) -> Color {
        let [x, y, z] = self.axes;
        match axis {
            Axis::X => x,
            Axis::Y => y,
            Axis::Z => z,
        }
    }
}

const fn axis_color(axis: [u8; 3]) -> Color {
    let [red, green, blue] = axis;
    Color::from_rgb8(red, green, blue)
}

const HOVERED: Color = opaque(canvas::HOVERED);
const SELECTED: Color = opaque(canvas::SELECTED);
const HOVERED_SELECTED: Color = Color::from_rgb8(150, 205, 255);
const LINE_HIGHLIGHTS: Highlights = Highlights {
    hovered: HOVERED,
    selected: SELECTED,
    hovered_selected: HOVERED_SELECTED,
};
const CURVE_WIDTH: f32 = 2.0;
const POINT_DIAMETER: f32 = 7.0;
const CANVAS_HOLE: Color = Color::from_rgb8(27, 28, 31);

pub const STANDARD: ScenePalette = ScenePalette {
    grid: Color::from_rgba8(210, 215, 225, 90),
    origin: Color::from_rgb8(235, 235, 235),
    plane_fill: Color::from_rgba8(120, 150, 200, 22),
    plane_edge: Color::from_rgba8(140, 170, 215, 150),
    axes: [
        axis_color(Axis::X.rgb()),
        axis_color(Axis::Y.rgb()),
        axis_color(Axis::Z.rgb()),
    ],
    sketch_horizontal_axis: Color::from_rgb8(226, 84, 84),
    sketch_vertical_axis: Color::from_rgb8(112, 196, 88),
    lines: LINE_HIGHLIGHTS,
    faces: LINE_HIGHLIGHTS,
    under_constrained: Look::plain(
        Color::from_rgb8(222, 224, 230),
        Color::from_rgb8(245, 245, 248),
    ),
    fully_constrained: Look::plain(
        Color::from_rgb8(104, 204, 120),
        Color::from_rgb8(146, 226, 158),
    ),
    conflicting: Look::plain(
        Color::from_rgb8(238, 78, 70),
        Color::from_rgb8(250, 108, 100),
    ),
    redundant: Look::plain(
        Color::from_rgb8(236, 132, 40),
        Color::from_rgb8(246, 158, 78),
    ),
    failed: Look::plain(
        Color::from_rgb8(214, 120, 110),
        Color::from_rgb8(232, 146, 136),
    ),
    projected: Look::plain(
        Color::from_rgb8(196, 136, 238),
        Color::from_rgb8(214, 166, 246),
    ),
    background: Look::plain(
        Color::from_rgb8(104, 108, 118),
        Color::from_rgb8(118, 122, 132),
    ),
    hole: CANVAS_HOLE,
    preview_curve: Color::from_rgb8(190, 150, 255),
    preview_point: Color::from_rgb8(214, 190, 255),
    trimmed_curve: Color::from_rgb8(255, 96, 84),
    cut_preview: Color::from_rgb8(232, 92, 80),
    cut_preview_edge: Color::from_rgb8(255, 132, 120),
    failed_body: Color::from_rgb8(200, 134, 124),
    outdated_body: Color::from_rgb8(182, 170, 130),
    background_body: Color::from_rgb8(92, 96, 104),
    body_edge: Color::from_rgb8(30, 32, 38),
    background_body_edge: Color::from_rgb8(62, 64, 70),
    drawing_face: Color::from_rgb8(236, 238, 242),
    chosen_region: translucent(canvas::SELECTED, CHOSEN_REGION_ALPHA),
    open_region: Color::from_rgba8(210, 214, 224, 26),
    closed_region: Color::from_rgba8(120, 170, 255, 52),
    revolve_axis: Color::from_rgb8(255, 150, 60),
    followed_edge: Color::from_rgb8(150, 200, 250),
    centre_of_mass: Color::from_rgb8(255, 196, 64),
    thread: Color::from_rgb8(28, 84, 150),
    bands: Bands {
        drafted: Color::from_rgb8(72, 168, 96),
        too_little_draft: Color::from_rgb8(236, 190, 52),
        undercut: Color::from_rgb8(214, 68, 62),
        too_tight: Color::from_rgb8(226, 72, 150),
    },
    datum_edge: Color::from_rgba8(236, 178, 92, 220),
    datum_fill: Color::from_rgba8(236, 178, 92, 26),
    failed_datum_edge: Color::from_rgba8(214, 120, 110, 220),
    failed_datum_fill: Color::from_rgba8(214, 120, 110, 26),
    curve_width: CURVE_WIDTH,
    heavy_curve_width: CURVE_WIDTH,
    body_edge_width: 1.5,
    point_diameter: POINT_DIAMETER,
    hole_diameter: 0.0,
    selection_widening: 1.0,
    troubled_edges_dashed: false,
};

pub const HIGH_CONTRAST: ScenePalette = ScenePalette {
    grid: Color::from_rgba8(200, 205, 215, 170),
    origin: Color::from_rgb8(255, 255, 255),
    plane_fill: Color::from_rgba8(150, 180, 225, 34),
    plane_edge: Color::from_rgb8(150, 180, 225),
    axes: [
        Color::from_rgb8(255, 110, 110),
        Color::from_rgb8(120, 220, 100),
        Color::from_rgb8(110, 170, 255),
    ],
    sketch_horizontal_axis: Color::from_rgb8(255, 110, 110),
    sketch_vertical_axis: Color::from_rgb8(120, 220, 100),
    lines: LINE_HIGHLIGHTS,
    faces: Highlights {
        hovered: Color::from_rgb8(120, 70, 0),
        selected: Color::from_rgb8(0, 70, 190),
        hovered_selected: Color::from_rgb8(0, 50, 140),
    },
    under_constrained: Look::formed(
        Color::from_rgb8(255, 255, 255),
        Weight::Regular,
        PointFill::Hollow,
    ),
    fully_constrained: Look::formed(
        Color::from_rgb8(110, 235, 140),
        Weight::Heavy,
        PointFill::Solid,
    ),
    conflicting: Look::formed(
        Color::from_rgb8(255, 120, 110),
        Weight::Heavy,
        PointFill::Solid,
    ),
    redundant: Look::formed(
        Color::from_rgb8(255, 170, 60),
        Weight::Heavy,
        PointFill::Solid,
    ),
    failed: Look::formed(
        Color::from_rgb8(255, 150, 140),
        Weight::Regular,
        PointFill::Solid,
    ),
    projected: Look::formed(
        Color::from_rgb8(215, 165, 255),
        Weight::Heavy,
        PointFill::Solid,
    ),
    background: Look::formed(
        Color::from_rgb8(150, 154, 164),
        Weight::Regular,
        PointFill::Solid,
    ),
    hole: CANVAS_HOLE,
    preview_curve: Color::from_rgb8(205, 170, 255),
    preview_point: Color::from_rgb8(225, 205, 255),
    trimmed_curve: Color::from_rgb8(255, 110, 100),
    cut_preview: Color::from_rgb8(232, 92, 80),
    cut_preview_edge: Color::from_rgb8(255, 140, 130),
    failed_body: Color::from_rgb8(200, 134, 124),
    outdated_body: Color::from_rgb8(182, 170, 130),
    background_body: Color::from_rgb8(52, 54, 60),
    body_edge: Color::from_rgb8(0, 0, 0),
    background_body_edge: Color::from_rgb8(150, 154, 164),
    drawing_face: Color::from_rgb8(255, 255, 255),
    chosen_region: translucent(canvas::SELECTED, CHOSEN_REGION_ALPHA),
    open_region: Color::from_rgba8(210, 214, 224, 40),
    closed_region: Color::from_rgba8(120, 170, 255, 70),
    revolve_axis: Color::from_rgb8(255, 160, 70),
    followed_edge: Color::from_rgb8(150, 200, 250),
    centre_of_mass: Color::from_rgb8(255, 208, 90),
    thread: Color::from_rgb8(0, 50, 130),
    bands: Bands {
        drafted: Color::from_rgb8(96, 214, 128),
        too_little_draft: Color::from_rgb8(255, 224, 70),
        undercut: Color::from_rgb8(255, 96, 88),
        too_tight: Color::from_rgb8(255, 110, 190),
    },
    datum_edge: Color::from_rgb8(245, 190, 100),
    datum_fill: Color::from_rgba8(245, 190, 100, 34),
    failed_datum_edge: Color::from_rgb8(255, 150, 140),
    failed_datum_fill: Color::from_rgba8(255, 150, 140, 34),
    curve_width: CURVE_WIDTH,
    heavy_curve_width: 3.5,
    body_edge_width: 2.0,
    point_diameter: 9.0,
    hole_diameter: 4.0,
    selection_widening: 2.0,
    troubled_edges_dashed: true,
};

#[cfg(test)]
mod tests {
    use egui::Color32;

    use super::*;
    use crate::{appearance::tests::contrast_ratio, body_appearance::DEFAULT_COLOUR};

    const VISIBLE: f32 = 3.0;

    fn channel(value: f64) -> u8 {
        (value * 255.0).round().clamp(0.0, 255.0) as u8
    }

    fn canvas() -> Color32 {
        let background = caditor_render::BACKGROUND;
        Color32::from_rgb(
            channel(background.r),
            channel(background.g),
            channel(background.b),
        )
    }

    fn over(color: Color, below: Color32) -> Color32 {
        let mix = |above: f32, under: u8| {
            let above = above * 255.0;
            (above * color.alpha + f32::from(under) * (1.0 - color.alpha)).round() as u8
        };
        Color32::from_rgb(
            mix(color.red, below.r()),
            mix(color.green, below.g()),
            mix(color.blue, below.b()),
        )
    }

    fn opaque32(color: Color) -> Color32 {
        over(color.with_alpha(1.0), Color32::BLACK)
    }

    fn assert_visible(what: &str, color: Color, below: Color32) {
        let ratio = contrast_ratio(over(color, below), below);
        assert!(
            ratio >= VISIBLE,
            "{what} on {below:?} is {ratio:.2}:1, below {VISIBLE}:1"
        );
    }

    #[test]
    fn the_hole_of_a_hollow_point_is_the_canvas() {
        assert_eq!(opaque32(CANVAS_HOLE), canvas());
    }

    #[test]
    fn high_contrast_sketch_geometry_stands_out_from_the_canvas_and_dimmed_bodies() {
        let palette = &HIGH_CONTRAST;
        let dimmed = opaque32(palette.background_body);
        let mut drawn_over_bodies = vec![
            ("hovered", palette.lines.hovered),
            ("selected", palette.lines.selected),
            ("hovered and selected", palette.lines.hovered_selected),
            ("sketch origin", palette.origin),
            ("sketch horizontal axis", palette.sketch_horizontal_axis),
            ("sketch vertical axis", palette.sketch_vertical_axis),
            ("preview curve", palette.preview_curve),
            ("preview point", palette.preview_point),
            ("trimmed curve", palette.trimmed_curve),
        ];
        for state in SketchState::ALL {
            let look = palette.look(state);
            drawn_over_bodies.push(("sketch curve", look.curve));
            drawn_over_bodies.push(("sketch point", look.point));
        }
        for (what, color) in drawn_over_bodies {
            assert_visible(what, color, canvas());
            assert_visible(what, color, dimmed);
        }
        for (what, color) in [
            ("grid", palette.grid),
            ("x axis", palette.axis(Axis::X)),
            ("y axis", palette.axis(Axis::Y)),
            ("z axis", palette.axis(Axis::Z)),
            ("plane edge", palette.plane_edge),
            ("datum", palette.datum_edge),
            ("failed datum", palette.failed_datum_edge),
            ("revolve axis", palette.revolve_axis),
            ("followed edge", palette.followed_edge),
            ("cut preview edge", palette.cut_preview_edge),
        ] {
            assert_visible(what, color, canvas());
        }
        assert_visible("dimmed body edge", palette.background_body_edge, dimmed);
    }

    #[test]
    fn high_contrast_edges_and_highlighted_faces_stand_out_on_a_body() {
        let palette = &HIGH_CONTRAST;
        let body = Color32::from_rgb(
            DEFAULT_COLOUR.red,
            DEFAULT_COLOUR.green,
            DEFAULT_COLOUR.blue,
        );
        for (what, color) in [
            ("edge", palette.body_edge),
            ("thread", palette.thread),
            ("hovered face", palette.faces.hovered),
            ("selected face", palette.faces.selected),
            ("hovered and selected face", palette.faces.hovered_selected),
        ] {
            assert_visible(what, color, body);
        }
        assert_visible(
            "edge on a drawing face",
            palette.body_edge,
            opaque32(palette.drawing_face),
        );
        assert!(palette.body_edge_width > STANDARD.body_edge_width);
    }

    #[test]
    fn the_centre_of_mass_marker_stands_out_from_its_outline_and_the_outline_from_a_body() {
        let body = Color32::from_rgb(
            DEFAULT_COLOUR.red,
            DEFAULT_COLOUR.green,
            DEFAULT_COLOUR.blue,
        );
        for palette in [&STANDARD, &HIGH_CONTRAST] {
            let outline = opaque32(palette.hole);
            for (what, color) in [
                ("centre of mass", palette.centre_of_mass),
                ("hovered centre of mass", palette.lines.hovered),
                ("selected centre of mass", palette.lines.selected),
                (
                    "hovered and selected centre of mass",
                    palette.lines.hovered_selected,
                ),
            ] {
                assert_visible(what, color, outline);
            }
            assert_visible("centre of mass outline", palette.hole, body);
        }
    }

    #[test]
    fn the_analysis_bands_stand_out_from_the_canvas_and_from_each_other() {
        for palette in [&STANDARD, &HIGH_CONTRAST] {
            let bands = palette.bands;
            let all = [
                ("drafted", bands.drafted),
                ("too little draft", bands.too_little_draft),
                ("undercut", bands.undercut),
                ("too tight", bands.too_tight),
            ];
            for (what, color) in all {
                assert_visible(what, color, canvas());
            }
            for (index, (what, color)) in all.iter().enumerate() {
                for (other, against) in all.iter().skip(index + 1) {
                    assert_ne!(color, against, "{what} and {other}");
                }
            }
        }
    }

    #[test]
    fn high_contrast_tells_sketch_states_apart_without_colour() {
        let palette = &HIGH_CONTRAST;
        let free = palette.look(SketchState::UnderConstrained);
        let fixed = palette.look(SketchState::FullyConstrained);

        assert_ne!(free.weight, fixed.weight);
        assert_ne!(free.fill, fixed.fill);
        assert_eq!(free.fill, PointFill::Hollow);
        for state in [
            SketchState::FullyConstrained,
            SketchState::Conflicting,
            SketchState::Redundant,
            SketchState::Projected,
        ] {
            assert_eq!(palette.look(state).weight, Weight::Heavy, "{state:?}");
            assert_eq!(palette.look(state).fill, PointFill::Solid, "{state:?}");
        }
        assert!(palette.curve_width(Weight::Heavy) >= palette.curve_width(Weight::Regular) + 1.0);
        assert!(
            palette.hole_diameter > 0.0 && palette.hole_diameter <= palette.point_diameter - 4.0
        );
        assert_visible("a hollow point's ring", free.point, opaque32(palette.hole));
    }

    #[test]
    fn the_standard_palette_draws_every_state_in_one_form() {
        for state in SketchState::ALL {
            let look = STANDARD.look(state);
            assert_eq!(
                (look.weight, look.fill),
                (Weight::Regular, PointFill::Solid)
            );
        }
        assert_eq!(
            STANDARD.curve_width(Weight::Heavy),
            STANDARD.curve_width(Weight::Regular)
        );
    }
}
