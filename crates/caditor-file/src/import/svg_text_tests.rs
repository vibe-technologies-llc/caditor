use std::collections::BTreeMap;

use caditor_geometry::Point2;
use caditor_kernel::{Profile, ProfileCurve, Selection};
use ttf_parser::{Face, GlyphId, OutlineBuilder, Rect, Tag};

use crate::import::{Drawing, DrawingCurve, ImportError, TextOutlines, parse_svg};

const INTER: &[u8] = include_bytes!("../../../caditor/assets/fonts/InterVariable.ttf");
const SVG_OPEN: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="100mm" viewBox="0 0 200 100">"#;
const JOINED: f64 = 1e-6;
const CLOSE: f64 = 1e-6;

fn svg(body: &str) -> Vec<u8> {
    format!("{SVG_OPEN}{body}</svg>").into_bytes()
}

fn read(body: &str) -> Drawing {
    parse_svg(&svg(body), TextOutlines::InFont(INTER)).unwrap()
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Bounds {
    low: Point2,
    high: Point2,
}

impl Bounds {
    fn of(points: impl IntoIterator<Item = Point2>) -> Self {
        points.into_iter().fold(
            Self {
                low: Point2::splat(f64::INFINITY),
                high: Point2::splat(f64::NEG_INFINITY),
            },
            |bounds, point| Self {
                low: bounds.low.min(point),
                high: bounds.high.max(point),
            },
        )
    }

    fn holds(&self, other: &Self) -> bool {
        self.low.cmplt(other.low).all() && other.high.cmplt(self.high).all()
    }

    fn width(&self) -> f64 {
        self.high.x - self.low.x
    }

    fn height(&self) -> f64 {
        self.high.y - self.low.y
    }
}

fn points(curve: &DrawingCurve) -> Vec<Point2> {
    match curve {
        DrawingCurve::Line { start, end } => vec![*start, *end],
        DrawingCurve::Spline { control_points } => control_points.clone(),
        other => panic!("a letter drew {other:?}"),
    }
}

fn key(point: Point2) -> (i64, i64) {
    (
        (point.x / JOINED).round() as i64,
        (point.y / JOINED).round() as i64,
    )
}

fn loops(drawing: &Drawing) -> Vec<Bounds> {
    let mut parent: Vec<usize> = (0..drawing.curves.len()).collect();
    fn root(parent: &mut [usize], mut index: usize) -> usize {
        while parent[index] != index {
            parent[index] = parent[parent[index]];
            index = parent[index];
        }
        index
    }
    let mut ends: BTreeMap<(i64, i64), Vec<usize>> = BTreeMap::new();
    for (index, curve) in drawing.curves.iter().enumerate() {
        let (start, end) = curve.ends().unwrap();
        ends.entry(key(start)).or_default().push(index);
        ends.entry(key(end)).or_default().push(index);
    }
    for (at, touching) in &ends {
        assert_eq!(touching.len(), 2, "an open end at {at:?}");
        let (first, second) = (
            root(&mut parent, touching[0]),
            root(&mut parent, touching[1]),
        );
        parent[first] = second;
    }
    let mut grouped: BTreeMap<usize, Vec<Point2>> = BTreeMap::new();
    for (index, curve) in drawing.curves.iter().enumerate() {
        let group = root(&mut parent, index);
        grouped.entry(group).or_default().extend(points(curve));
    }
    let mut bounds: Vec<Bounds> = grouped.into_values().map(Bounds::of).collect();
    bounds.sort_by(|a, b| a.low.x.total_cmp(&b.low.x));
    bounds
}

fn whole(drawing: &Drawing) -> Bounds {
    Bounds::of(drawing.curves.iter().flat_map(points))
}

#[derive(Default)]
struct Extent {
    points: Vec<Point2>,
}

impl OutlineBuilder for Extent {
    fn move_to(&mut self, x: f32, y: f32) {
        self.points.push(Point2::new(x.into(), y.into()));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.move_to(x, y);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.move_to(x1, y1);
        self.move_to(x, y);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.move_to(x1, y1);
        self.move_to(x2, y2);
        self.move_to(x, y);
    }

    fn close(&mut self) {}
}

fn regular() -> Face<'static> {
    let mut face = Face::parse(INTER, 0).unwrap();
    face.set_variation(Tag::from_bytes(b"wght"), 400.0).unwrap();
    face
}

fn glyph(face: &Face<'_>, character: char) -> GlyphId {
    face.glyph_index(character).unwrap()
}

fn glyph_extent(face: &Face<'_>, character: char) -> Bounds {
    let mut extent = Extent::default();
    let _: Option<Rect> = face.outline_glyph(glyph(face, character), &mut extent);
    Bounds::of(extent.points)
}

fn advance(face: &Face<'_>, character: char) -> f64 {
    f64::from(face.glyph_hor_advance(glyph(face, character)).unwrap())
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < CLOSE,
        "{actual} is not {expected}"
    );
}

#[test]
fn a_letter_imports_as_closed_outlines_with_its_hole_inside() {
    let drawing = read(r#"<text x="10" y="50" font-size="20">o</text>"#);

    let face = regular();
    let scale = 20.0 / f64::from(face.units_per_em());
    let extent = glyph_extent(&face, 'o');
    let outlines = loops(&drawing);

    assert_eq!(outlines.len(), 2, "{outlines:?}");
    assert!(outlines[0].holds(&outlines[1]), "{outlines:?}");
    let bounds = whole(&drawing);
    assert_close(bounds.low.x, 10.0 + extent.low.x * scale);
    assert_close(bounds.high.x, 10.0 + extent.high.x * scale);
    assert_close(bounds.low.y, -50.0 + extent.low.y * scale);
    assert_close(bounds.high.y, -50.0 + extent.high.y * scale);
    assert!(drawing.notes.is_empty(), "{:?}", drawing.notes);
}

#[test]
fn every_letter_of_a_word_closes_and_holes_fall_inside_their_letters() {
    let drawing = read(r#"<text x="0" y="50" font-size="30">aeo B</text>"#);

    let outlines = loops(&drawing);

    assert_eq!(outlines.len(), 9, "{outlines:?}");
    for (outer, hole) in [(0, 1), (2, 3), (4, 5)] {
        assert!(outlines[outer].holds(&outlines[hole]), "{outlines:?}");
    }
    assert!(outlines[6].holds(&outlines[7]) && outlines[6].holds(&outlines[8]));
}

#[test]
fn the_anchor_shifts_each_chunk_by_its_advance() {
    let at = |anchor: &str| {
        whole(&read(&format!(
            r#"<text x="100" y="50" font-size="20" text-anchor="{anchor}">II</text>"#
        )))
    };

    let (start, middle, end) = (at("start"), at("middle"), at("end"));

    let face = regular();
    let scale = 20.0 / f64::from(face.units_per_em());
    let width = advance(&face, 'I') * 2.0 * scale;
    assert_close(start.low.x - end.low.x, width);
    assert_close(start.low.x - middle.low.x, width / 2.0);
    assert_close(start.low.y, end.low.y);
}

#[test]
fn per_letter_positions_place_each_letter() {
    let drawing = read(r#"<text x="10 40 70" y="50" dy="0 5" font-size="20">III</text>"#);

    let outlines = loops(&drawing);

    assert_eq!(outlines.len(), 3);
    assert_close(outlines[1].low.x - outlines[0].low.x, 30.0);
    assert_close(outlines[2].low.x - outlines[1].low.x, 30.0);
    assert_close(outlines[0].low.y - outlines[1].low.y, 5.0);
    assert_close(outlines[2].low.y, outlines[1].low.y);
}

#[test]
fn spans_take_their_own_positions_size_and_anchor_through_the_cascade() {
    let drawing = read(
        r#"<style>.big { font-size: 40px }</style>
           <g style="font-size:20px">
             <text x="10" y="50">I<tspan class="big" x="100" y="80" text-anchor="end">I</tspan></text>
           </g>"#,
    );

    let face = regular();
    let small = 20.0 / f64::from(face.units_per_em());
    let extent = glyph_extent(&face, 'I');
    let outlines = loops(&drawing);

    assert_eq!(outlines.len(), 2);
    assert_close(outlines[0].height(), extent.height() * small);
    assert_close(outlines[1].height(), extent.height() * small * 2.0);
    assert_close(
        outlines[1].low.x,
        100.0 - advance(&face, 'I') * small * 2.0 + extent.low.x * small * 2.0,
    );
    assert_close(outlines[1].low.y, -80.0 + extent.low.y * small * 2.0);
}

#[test]
fn kerning_and_collapsed_spaces_set_letters_as_the_font_does() {
    let kerned = loops(&read(r#"<text x="0" y="50" font-size="100">AV</text>"#));
    let spaced = loops(&read(
        "<text x=\"0\" y=\"50\" font-size=\"20\">I \n\t  I</text>",
    ));
    let single = loops(&read(r#"<text x="0" y="50" font-size="20">I I</text>"#));

    let face = regular();
    let scale = 100.0 / f64::from(face.units_per_em());
    let unkerned = advance(&face, 'A') * scale + glyph_extent(&face, 'V').low.x * scale;
    assert!(kerned[1].low.x < unkerned - 0.5, "{kerned:?}");
    assert_eq!(spaced.len(), 2);
    assert_close(spaced[1].low.x, single[1].low.x);
}

#[test]
fn transforms_and_weights_reach_the_letters() {
    let regular_letter = whole(&read(r#"<text y="50" font-size="20">I</text>"#));
    let bold = whole(&read(
        r#"<text y="50" font-size="20" font-weight="bold">I</text>"#,
    ));
    let moved = whole(&read(
        r#"<g transform="translate(30 0)"><text y="50" font-size="20">I</text></g>"#,
    ));

    assert!(bold.width() > regular_letter.width() * 1.3, "{bold:?}");
    assert_close(moved.low.x - regular_letter.low.x, 30.0);
}

#[test]
fn substituted_families_and_italics_are_noted_once() {
    let drawing = read(
        r#"<text y="20" font-family="'Helvetica Neue', Arial, sans-serif">I</text>
           <text y="40" style="font: italic 12px Helvetica Neue">I</text>
           <text y="60" font-family="Inter">I</text>
           <text y="80" font-family="sans-serif">I</text>"#,
    );

    let family_notes: Vec<&String> = drawing
        .notes
        .iter()
        .filter(|note| note.contains("drawn in Inter"))
        .collect();

    assert_eq!(family_notes.len(), 1, "{:?}", drawing.notes);
    assert!(
        family_notes[0].contains("Helvetica Neue"),
        "{family_notes:?}"
    );
    assert!(!family_notes[0].contains("Arial"), "{family_notes:?}");
    assert!(
        drawing
            .notes
            .iter()
            .any(|note| note.starts_with("1 text in italic was drawn upright")),
        "{:?}",
        drawing.notes
    );
}

#[test]
fn letters_the_font_lacks_and_text_on_paths_are_left_out_with_notes() {
    let drawing = read(
        r##"<defs><path id="p" d="M 0 0 L 100 0"/></defs>
            <text y="50" font-size="20">I&#x4E2D;<textPath href="#p">I</textPath></text>"##,
    );

    assert_eq!(loops(&drawing).len(), 1);
    assert!(
        drawing
            .notes
            .iter()
            .any(|note| note.starts_with("1 character Inter has no letter for was left out")),
        "{:?}",
        drawing.notes
    );
    assert!(
        drawing
            .notes
            .iter()
            .any(|note| note.starts_with("1 text laid along a path was left out")),
        "{:?}",
        drawing.notes
    );
}

#[test]
fn without_font_data_text_is_left_out_with_a_note() {
    let text = svg(r#"<line x1="0" y1="0" x2="10" y2="0"/><text y="50">Label</text>"#);

    let drawing = parse_svg(&text, TextOutlines::LeftOut).unwrap();
    let unusable = parse_svg(&text, TextOutlines::InFont(b"not a font")).unwrap();
    let only_text = parse_svg(&svg(r#"<text y="50">Label</text>"#), TextOutlines::LeftOut);

    for drawing in [&drawing, &unusable] {
        assert_eq!(drawing.curves.len(), 1);
        assert!(
            drawing
                .notes
                .iter()
                .any(|note| note.starts_with("1 text was left out")),
            "{:?}",
            drawing.notes
        );
    }
    assert!(matches!(only_text, Err(ImportError::Empty { .. })));
}

#[test]
fn hidden_and_unpainted_spans_draw_nothing_but_keep_their_room() {
    let drawing = read(
        r#"<text x="0" y="50" font-size="20">I<tspan visibility="hidden">I</tspan><tspan fill="none" stroke="none">I</tspan>I<tspan display="none">IIII</tspan></text>"#,
    );
    let plain = loops(&read(r#"<text x="0" y="50" font-size="20">IIII</text>"#));

    let outlines = loops(&drawing);

    assert_eq!(outlines.len(), 2);
    assert_close(outlines[1].low.x, plain[3].low.x);
}

fn profile_curve(entity: u64, curve: &DrawingCurve) -> ProfileCurve {
    match curve {
        DrawingCurve::Line { start, end } => ProfileCurve::line(entity, *start, *end),
        DrawingCurve::Spline { control_points } => {
            let count = control_points.len();
            let knots = [vec![0.0; count], vec![1.0; count]].concat();
            ProfileCurve::spline(entity, count - 1, knots, control_points.clone())
        }
        other => panic!("a letter drew {other:?}"),
    }
}

#[test]
fn overlapping_strokes_merge_so_the_default_regions_fill_the_whole_letter() {
    let drawing = read(r#"<text x="0" y="0" font-size="2048">e</text>"#);

    let curves: Vec<ProfileCurve> = (0..)
        .zip(&drawing.curves)
        .map(|(entity, curve)| profile_curve(entity, curve))
        .collect();
    let profile = Profile::new(&curves).unwrap();
    let chosen = profile.select(&Selection::EvenDepth).unwrap();
    let inside = |x: f64, y: f64| {
        chosen
            .iter()
            .any(|region| region.contains(Point2::new(x, y)))
    };

    assert_eq!(profile.regions().len(), 2);
    assert_eq!(chosen.len(), 1);
    assert_eq!(chosen[0].holes().len(), 1);
    assert!(inside(256.0, 574.0), "where the bar overlaps the bowl");
    assert!(inside(600.0, 574.0), "the bar");
    assert!(!inside(600.0, 800.0), "the eye");
    assert!(!inside(600.0, 300.0), "the mouth");
}
