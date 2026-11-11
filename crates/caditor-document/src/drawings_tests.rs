use std::collections::BTreeMap;

use caditor_expression::{Expression, ParameterId};
use caditor_geometry::{Aabb2, Point2, Point3};
use caditor_kernel::{Curve2, ViewPiece, Visibility};

use crate::{
    combine_tests::{Pair, evaluate, pair},
    *,
};

fn new_drawing(document: &mut Document) -> DrawingId {
    let (transaction, id) = document.adding_drawing(Vec::new()).unwrap();
    document.apply(transaction).unwrap();
    id
}

fn set(drawings: Drawings) -> Transaction {
    Transaction::single(
        "Drawings",
        Edit::SetDrawings {
            drawings: Box::new(drawings),
        },
    )
}

fn view_lines(pair: &Pair, view: StandardView, drawing: &Drawing) -> ViewLines {
    let evaluation = evaluate(&pair.document, &mut Recompute::default());
    let bodies = drawn_bodies(&pair.document, &evaluation, drawing);
    drawing_view(view, &bodies.solids()).unwrap()
}

fn extent(lines: &ViewLines) -> (f64, f64) {
    let size = lines.bounds.unwrap().size();
    (size.x, size.y)
}

fn close(left: f64, right: f64) -> bool {
    (left - right).abs() < 1e-6
}

#[test]
fn a_new_drawing_is_undoable_content_with_a_numbered_name() {
    let pair = pair();
    let mut editor = Editor::new(pair.document.clone());

    let (transaction, id) = editor.document().adding_drawing(Vec::new()).unwrap();
    let touched = transaction.touched();
    editor.apply(transaction).unwrap();
    let with_drawing = editor.document().clone();
    editor.undo().unwrap();

    assert!(touched.drawings);
    assert_eq!(with_drawing.drawing(id).unwrap().name, "Drawing 1");
    assert!(!with_drawing.same_content(&pair.document));
    assert!(editor.document().same_content(&pair.document));
    assert_eq!(editor.redo_label(), Some("New drawing"));
}

#[test]
fn drawing_numbers_are_never_reused() {
    let mut document = pair().document;

    let first = new_drawing(&mut document);
    let deletion = document.deleting_drawing(first).unwrap();
    document.apply(deletion).unwrap();
    let second = new_drawing(&mut document);

    assert!(second > first);
    assert!(document.drawing(first).is_none());
    assert_eq!(document.drawings().drawings.len(), 1);
}

#[test]
fn undoing_a_drawing_keeps_its_number_taken() {
    let pair = pair();
    let mut editor = Editor::new(pair.document.clone());

    let (transaction, first) = editor.document().adding_drawing(Vec::new()).unwrap();
    editor.apply(transaction).unwrap();
    editor.undo().unwrap();
    let (transaction, second) = editor.document().adding_drawing(Vec::new()).unwrap();

    assert!(transaction.touched().drawings);
    assert!(second > first);
}

#[test]
fn names_scales_and_counts_are_checked() {
    let mut document = pair().document;
    let id = new_drawing(&mut document);
    let other = new_drawing(&mut document);

    let empty = document.changing_drawing(id, "Rename", |drawing| drawing.name = " \n ".into());
    let taken = document.changing_drawing(id, "Rename", |drawing| {
        drawing.name = "drawing 2".into();
    });
    let long = document.changing_drawing(id, "Rename", |drawing| {
        drawing.name = "x".repeat(MAX_DRAWING_NAME_CHARS + 1);
    });
    let scale = document.changing_drawing(other, "Scale", |drawing| {
        drawing.scale = DrawingScale::Ratio(ScaleRatio::new(0, 2));
    });
    let mut many = document.drawings().clone();
    for number in 0..MAX_DRAWINGS {
        many.drawings.push(Drawing::new(
            DrawingId::from_raw(100 + number as u64),
            format!("Extra {number}"),
        ));
    }

    assert_eq!(empty.unwrap_err(), EditError::DrawingNameEmpty);
    assert_eq!(
        taken.unwrap_err(),
        EditError::DrawingNameTaken("Drawing 2".into())
    );
    assert!(matches!(
        long.unwrap_err(),
        EditError::DrawingNameTooLong { .. }
    ));
    assert_eq!(
        scale.unwrap_err(),
        EditError::DrawingScaleOutOfRange { sheet: 0, model: 2 }
    );
    assert_eq!(
        document.apply(set(many)).unwrap_err(),
        EditError::TooManyDrawings
    );
}

#[test]
fn restoring_an_earlier_version_restores_its_drawings() {
    let pair = pair();
    let mut later = pair.document.clone();
    let id = new_drawing(&mut later);
    let renamed = later
        .changing_drawing(id, "Rename", |drawing| drawing.name = "Workshop".into())
        .unwrap();
    later.apply(renamed).unwrap();

    let mut current = later.clone();
    current
        .apply(current.transaction_to(&pair.document, "Restore"))
        .unwrap();
    let mut back = current.clone();
    back.apply(back.transaction_to(&later, "Restore")).unwrap();

    assert!(current.drawings().is_empty());
    assert_eq!(back.drawing(id).unwrap().name, "Workshop");
}

#[test]
fn a_drawing_shows_every_shown_body_unless_bodies_are_chosen() {
    let pair = pair();
    let mut document = pair.document.clone();
    document
        .apply(Transaction::single(
            "Hide",
            Edit::SetFeatureHidden {
                id: pair.peg,
                hidden: true,
            },
        ))
        .unwrap();
    let evaluation = evaluate(&document, &mut Recompute::default());
    let mut drawing = Drawing::new(DrawingId::from_raw(0), "Drawing 1".into());

    let every = drawn_bodies(&document, &evaluation, &drawing);
    drawing.bodies = vec![pair.peg, FeatureId::from_raw(999)];
    let chosen = drawn_bodies(&document, &evaluation, &drawing);

    assert_eq!(
        every
            .results
            .iter()
            .map(|(body, _)| *body)
            .collect::<Vec<_>>(),
        vec![pair.plate]
    );
    assert_eq!(
        chosen
            .results
            .iter()
            .map(|(body, _)| *body)
            .collect::<Vec<_>>(),
        vec![pair.peg]
    );
    assert_eq!(chosen.missing, vec![FeatureId::from_raw(999)]);
}

#[test]
fn standard_views_project_the_block_along_the_model_axes() {
    let pair = pair();
    let mut drawing = Drawing::new(DrawingId::from_raw(0), "Drawing 1".into());
    drawing.bodies = vec![pair.plate];

    let front = view_lines(&pair, StandardView::Front, &drawing);
    let top = view_lines(&pair, StandardView::Top, &drawing);
    let right = view_lines(&pair, StandardView::Right, &drawing);
    let left = view_lines(&pair, StandardView::Left, &drawing);

    assert!(close(extent(&front).0, 20.0) && close(extent(&front).1, 4.0));
    assert!(close(extent(&top).0, 20.0) && close(extent(&top).1, 10.0));
    assert!(close(extent(&right).0, 10.0) && close(extent(&right).1, 4.0));
    assert!(close(front.bounds.unwrap().min().x, 0.0));
    assert!(close(right.bounds.unwrap().min().x, 0.0));
    assert!(close(left.bounds.unwrap().max().x, 0.0));
    assert_eq!(front.drawn_pieces(false).count(), 4);
}

#[test]
fn edges_behind_another_body_are_hidden_and_can_be_left_out() {
    let pair = pair();
    let drawing = Drawing::new(DrawingId::from_raw(0), "Drawing 1".into());

    let top = view_lines(&pair, StandardView::Top, &drawing);
    let hidden = top
        .pieces
        .iter()
        .filter(|piece| piece.visibility == Visibility::Hidden)
        .count();

    assert!(hidden > 0);
    assert_eq!(top.drawn_pieces(true).count(), top.pieces.len());
    assert_eq!(top.drawn_pieces(false).count(), top.pieces.len() - hidden);
}

#[test]
fn a_view_of_no_bodies_fails_in_words() {
    assert_eq!(
        drawing_view(StandardView::Front, &[]).unwrap_err(),
        DrawingViewError::NoBodies
    );
}

fn extents_of_a_box(x: f64, y: f64, z: f64) -> BTreeMap<DrawnView, Aabb2> {
    let rectangle = |width: f64, height: f64| {
        Aabb2::from_point(Point2::ZERO).including(Point2::new(width, height))
    };
    BTreeMap::from([
        (DrawnView::Standard(StandardView::Front), rectangle(x, z)),
        (DrawnView::Standard(StandardView::Top), rectangle(x, y)),
        (
            DrawnView::Standard(StandardView::Right),
            Aabb2::from_point(Point2::ZERO).including(Point2::new(y, z)),
        ),
        (
            DrawnView::Standard(StandardView::Left),
            Aabb2::from_point(Point2::new(-y, 0.0)).including(Point2::new(0.0, z)),
        ),
        (
            DrawnView::Standard(StandardView::Isometric),
            rectangle(x, z),
        ),
    ])
}

fn placed(layout: &SheetPlan, view: StandardView) -> PlacedView {
    placed_as(layout, DrawnView::Standard(view))
}

fn placed_as(layout: &SheetPlan, view: DrawnView) -> PlacedView {
    *layout
        .views
        .iter()
        .find(|placed| placed.view == view)
        .unwrap()
}

#[test]
fn fitting_takes_the_largest_preferred_scale_the_sheet_holds() {
    let drawing = Drawing::new(DrawingId::from_raw(0), "Drawing 1".into());
    let extents = extents_of_a_box(100.0, 60.0, 40.0);

    let layout = lay_out_sheet(
        &drawing,
        &TitleFields::default(),
        &extents,
        &BTreeMap::new(),
    );

    assert_eq!(layout.scale, ScaleRatio::new(1, 1));
    assert!(layout.fits);
    assert_eq!((layout.width, layout.height), (420.0, 297.0));
    for view in &layout.views {
        let bounds = extents.get(&view.view).unwrap();
        assert!(layout.view_area.contains(view.on_sheet(bounds.min())));
        assert!(layout.view_area.contains(view.on_sheet(bounds.max())));
    }
}

#[test]
fn views_line_up_with_the_front_view_by_the_projection_chosen() {
    let mut drawing = Drawing::new(DrawingId::from_raw(0), "Drawing 1".into());
    let extents = extents_of_a_box(100.0, 60.0, 40.0);

    let first = lay_out_sheet(
        &drawing,
        &TitleFields::default(),
        &extents,
        &BTreeMap::new(),
    );
    drawing.projection = Projection::ThirdAngle;
    let third = lay_out_sheet(
        &drawing,
        &TitleFields::default(),
        &extents,
        &BTreeMap::new(),
    );

    for layout in [&first, &third] {
        let front = placed(layout, StandardView::Front);
        let top = placed(layout, StandardView::Top);
        let right = placed(layout, StandardView::Right);
        assert!(close(front.origin.x, top.origin.x));
        assert!(close(front.origin.y, right.origin.y));
    }
    assert!(
        placed(&first, StandardView::Top).origin.y < placed(&first, StandardView::Front).origin.y
    );
    assert!(
        placed(&first, StandardView::Right).origin.x < placed(&first, StandardView::Front).origin.x
    );
    assert!(
        placed(&third, StandardView::Top).origin.y > placed(&third, StandardView::Front).origin.y
    );
    assert!(
        placed(&third, StandardView::Right).origin.x > placed(&third, StandardView::Front).origin.x
    );
}

#[test]
fn a_scale_too_large_for_the_sheet_says_so() {
    let mut drawing = Drawing::new(DrawingId::from_raw(0), "Drawing 1".into());
    drawing.scale = DrawingScale::Ratio(ScaleRatio::new(10, 1));
    let extents = extents_of_a_box(100.0, 60.0, 40.0);

    let layout = lay_out_sheet(
        &drawing,
        &TitleFields::default(),
        &extents,
        &BTreeMap::new(),
    );
    drawing.size = SheetSize::A4;
    drawing.scale = DrawingScale::Fit;
    let small = lay_out_sheet(
        &drawing,
        &TitleFields::default(),
        &extents,
        &BTreeMap::new(),
    );

    assert!(!layout.fits);
    assert_eq!(layout.scale, ScaleRatio::new(10, 1));
    assert!(small.fits);
    assert_eq!(small.scale, ScaleRatio::new(1, 2));
}

#[test]
fn the_title_block_names_the_model_and_cuts_what_does_not_fit() {
    let drawing = Drawing::new(DrawingId::from_raw(0), "Drawing 1".into());
    let properties = ModelProperties {
        part_number: "P-100".into(),
        author: "x".repeat(200),
        ..ModelProperties::default()
    };

    let fields = TitleFields::of(&properties, "Bracket", &drawing);
    let layout = lay_out_sheet(&drawing, &fields, &BTreeMap::new(), &BTreeMap::new());
    let contents: Vec<&str> = layout
        .texts
        .iter()
        .map(|text| text.content.as_str())
        .collect();

    assert_eq!(fields.title, "Bracket");
    assert!(contents.contains(&"Bracket"));
    assert!(contents.contains(&"P-100"));
    assert!(contents.contains(&"1:1"));
    assert!(contents.contains(&"First angle"));
    assert!(contents.iter().any(|content| content.ends_with('…')));
    assert!(
        layout
            .texts
            .iter()
            .all(|text| layout.frame.contains(text.at))
    );
}

fn section(letter: char, plane: SectionPlane, place: SectionPlace) -> SectionView {
    SectionView {
        letter,
        parent: StandardView::Front,
        plane,
        place,
        reversed: false,
    }
}

fn section_lines(pair: &Pair, section: &SectionView) -> Result<ViewLines, DrawingViewError> {
    let drawing = Drawing::new(DrawingId::from_raw(0), "Drawing 1".into());
    let evaluation = evaluate(&pair.document, &mut Recompute::default());
    let bodies = drawn_bodies(&pair.document, &evaluation, &drawing);
    let place = section
        .place
        .resolve(&ParameterValues::evaluate(&pair.document))?;
    section_view(section, place, &bodies.solids())
}

#[test]
fn a_section_view_is_added_undoably_with_the_next_free_letter() {
    let pair = pair();
    let mut editor = Editor::new(pair.document.clone());
    let id = {
        let (transaction, id) = editor.document().adding_drawing(Vec::new()).unwrap();
        editor.apply(transaction).unwrap();
        id
    };

    let (first, a) = editor.document().adding_section_view(id).unwrap();
    editor.apply(first).unwrap();
    let (second, b) = editor.document().adding_section_view(id).unwrap();
    editor.apply(second).unwrap();
    let sections = editor.document().drawing(id).unwrap().sections.clone();
    editor.undo().unwrap();

    assert_eq!((a, b), ('A', 'B'));
    assert_eq!(sections.len(), 2);
    assert_eq!(sections[0].parent, StandardView::Front);
    assert_eq!(sections[0].plane, SectionPlane::Yz);
    assert_eq!(sections[0].place, SectionPlace::Centre);
    assert_eq!(sections[0].label(), "A\u{2013}A");
    assert_eq!(editor.document().drawing(id).unwrap().sections.len(), 1);
    assert_eq!(editor.redo_label(), Some("New section view"));
}

#[test]
fn section_views_are_checked() {
    let mut document = pair().document;
    let id = new_drawing(&mut document);
    let with = |change: SectionView| {
        document.changing_drawing(id, "Section", |drawing| drawing.sections = vec![change])
    };
    let facing = section('A', SectionPlane::Xz, SectionPlace::Centre);
    let unknown = section(
        'A',
        SectionPlane::Yz,
        SectionPlace::Offset(Expression::Parameter(ParameterId::from_raw(99))),
    );
    let unlettered = section('O', SectionPlane::Yz, SectionPlace::Centre);
    let twice = document.changing_drawing(id, "Sections", |drawing| {
        drawing.sections = vec![
            section('A', SectionPlane::Yz, SectionPlace::Centre),
            section('A', SectionPlane::Xy, SectionPlace::offset(2.0)),
        ];
    });
    let too_many = document.changing_drawing(id, "Sections", |drawing| {
        drawing.sections = (0..=MAX_SECTION_VIEWS)
            .map(|_| section('A', SectionPlane::Yz, SectionPlace::Centre))
            .collect();
    });

    assert!(matches!(
        with(facing).unwrap_err(),
        EditError::SectionPlaneFacesParent { letter: 'A', .. }
    ));
    assert_eq!(with(unknown).unwrap_err(), EditError::MissingParameter);
    assert_eq!(
        with(unlettered).unwrap_err(),
        EditError::ViewLetterUnusable('O')
    );
    assert_eq!(twice.unwrap_err(), EditError::ViewLetterRepeated('A'));
    assert!(matches!(
        too_many.unwrap_err(),
        EditError::DrawingTooManySections { .. }
    ));
    assert!(with(section('A', SectionPlane::Xy, SectionPlace::offset(2.0))).is_ok());
}

#[test]
fn a_section_cuts_the_bodies_and_hatches_what_the_plane_cuts() {
    let pair = pair();
    let through_both = section('A', SectionPlane::Yz, SectionPlace::offset(17.0));
    let through_plate = section('B', SectionPlane::Yz, SectionPlace::offset(5.0));

    let both = section_lines(&pair, &through_both).unwrap();
    let plate = section_lines(&pair, &through_plate).unwrap();
    let cut = plate.section.as_ref().unwrap();
    let hatched = Aabb2::from_points(
        cut.hatching[0]
            .edges
            .iter()
            .flat_map(|(start, end)| [*start, *end]),
    )
    .unwrap();

    assert_eq!(both.view, DrawnView::Section('A'));
    assert_eq!(both.hatching().len(), 2);
    assert!(close(both.hatching()[0].angle, HATCH_ANGLES[0]));
    assert!(close(both.hatching()[1].angle, HATCH_ANGLES[1]));
    assert_eq!(plate.hatching().len(), 1);
    assert!(close(cut.cutting.origin().x, 5.0));
    assert!(close(cut.cutting.normal().x, -1.0));
    assert!(close(hatched.size().x, 10.0) && close(hatched.size().y, 4.0));
    assert!(
        plate
            .drawn_pieces(true)
            .all(|piece| piece.visibility == Visibility::Visible)
    );
    assert!(close(extent(&plate).0, 10.0) && close(extent(&plate).1, 4.0));
}

#[test]
fn a_section_through_the_centre_follows_the_bodies_and_one_missing_them_says_so() {
    let pair = pair();
    let centred = section('A', SectionPlane::Xy, SectionPlace::Centre);
    let missing = section('B', SectionPlane::Yz, SectionPlace::offset(60.0));
    let reversed = SectionView {
        reversed: true,
        ..section('C', SectionPlane::Yz, SectionPlace::offset(17.0))
    };

    let lines = section_lines(&pair, &centred).unwrap();
    let back = section_lines(&pair, &reversed).unwrap();

    assert!(close(lines.section.unwrap().cutting.origin().z, 3.0));
    assert_eq!(
        section_lines(&pair, &missing).unwrap_err(),
        DrawingViewError::PlaneMisses
    );
    assert!(close(back.section.unwrap().cutting.normal().x, 1.0));
    assert_eq!(
        section_view(&centred, ResolvedPlace::Centre, &[]).unwrap_err(),
        DrawingViewError::NoBodies
    );
}

#[test]
fn hatching_fills_the_cut_face_at_one_spacing_on_the_sheet() {
    let square = [
        (Point2::new(0.0, 0.0), Point2::new(10.0, 0.0)),
        (Point2::new(10.0, 0.0), Point2::new(10.0, 10.0)),
        (Point2::new(10.0, 10.0), Point2::new(0.0, 10.0)),
        (Point2::new(0.0, 10.0), Point2::new(0.0, 0.0)),
    ];
    let region = HatchRegion {
        edges: square.to_vec(),
        angle: HATCH_ANGLES[0],
    };
    let at = |scale: f64| PlacedView {
        view: DrawnView::Section('A'),
        origin: Point2::new(100.0, 50.0),
        scale,
    };
    let inside = |placed: &PlacedView, point: Point2| {
        let local = (point - placed.origin) / placed.scale;
        (-1e-9..=10.0 + 1e-9).contains(&local.x) && (-1e-9..=10.0 + 1e-9).contains(&local.y)
    };

    let full = at(1.0);
    let doubled = at(2.0);
    let lines = hatch_lines(&region, &full);
    let more = hatch_lines(&region, &doubled);

    let diagonal = 10.0 * std::f64::consts::SQRT_2;
    assert!(lines.len() >= (diagonal / HATCH_SPACING) as usize);
    assert!(lines.len() <= (diagonal / HATCH_SPACING) as usize + 1);
    assert!(more.len() >= 2 * lines.len() - 1);
    for (start, end) in &lines {
        let direction = (*end - *start).normalize();
        assert!(inside(&full, *start) && inside(&full, *end));
        assert!(close(direction.x.abs(), direction.y.abs()));
    }
}

#[test]
fn section_views_stand_beside_the_others_labelled_and_cut_on_their_parent() {
    let mut drawing = Drawing::new(DrawingId::from_raw(0), "Drawing 1".into());
    drawing.sections = vec![
        section('A', SectionPlane::Yz, SectionPlace::Centre),
        SectionView {
            parent: StandardView::Left,
            ..section('B', SectionPlane::Xz, SectionPlace::Centre)
        },
    ];
    let mut extents = extents_of_a_box(100.0, 60.0, 40.0);
    extents.insert(
        DrawnView::Section('A'),
        Aabb2::from_point(Point2::ZERO).including(Point2::new(60.0, 40.0)),
    );
    extents.insert(
        DrawnView::Section('B'),
        Aabb2::from_point(Point2::ZERO).including(Point2::new(100.0, 40.0)),
    );
    let cuts = BTreeMap::from([
        (
            'A',
            caditor_geometry::Plane::new(
                caditor_geometry::Point3::new(50.0, 0.0, 0.0),
                -caditor_geometry::Vector3::X,
            )
            .unwrap(),
        ),
        (
            'B',
            caditor_geometry::Plane::new(
                caditor_geometry::Point3::new(0.0, 30.0, 0.0),
                caditor_geometry::Vector3::Y,
            )
            .unwrap(),
        ),
    ]);

    let layout = lay_out_sheet(&drawing, &TitleFields::default(), &extents, &cuts);
    let front = placed(&layout, StandardView::Front);
    let isometric = placed(&layout, StandardView::Isometric);
    let first = placed_as(&layout, DrawnView::Section('A'));
    let second = placed_as(&layout, DrawnView::Section('B'));
    let line = layout.cutting.first().unwrap();
    let contents: Vec<&str> = layout
        .texts
        .iter()
        .map(|text| text.content.as_str())
        .collect();

    assert!(layout.fits);
    assert!(first.origin.x > isometric.origin.x);
    assert!(first.origin.y > second.origin.y);
    for view in [first, second] {
        let bounds = extents.get(&view.view).unwrap();
        assert!(layout.view_area.contains(view.on_sheet(bounds.min())));
        assert!(layout.view_area.contains(view.on_sheet(bounds.max())));
    }
    assert!(contents.contains(&"A\u{2013}A"));
    assert!(contents.contains(&"B\u{2013}B"));
    assert_eq!(contents.iter().filter(|text| **text == "A").count(), 2);
    assert_eq!(layout.cutting.len(), 1);
    assert_eq!(line.letter, 'A');
    assert!(close(
        line.chain.0.x,
        front.on_sheet(Point2::new(50.0, 0.0)).x
    ));
    assert!(close(line.chain.0.x, line.chain.1.x));
    assert_eq!(layout.unmarked, vec!['B']);
}

fn offset_parameter(document: &mut Document, expression: &str) -> ParameterId {
    let mut transaction = document.transaction("Offset");
    let parsed = document.parse(expression).unwrap();
    let id = transaction.add_parameter("cut", parsed);
    document.apply(transaction.finish()).unwrap();
    id
}

fn cut_with(document: &mut Document, id: DrawingId, parameter: ParameterId) {
    let change = document
        .changing_drawing(id, "Section", |drawing| {
            drawing.sections = vec![section(
                'A',
                SectionPlane::Yz,
                SectionPlace::Offset(Expression::Parameter(parameter)),
            )];
        })
        .unwrap();
    document.apply(change).unwrap();
}

#[test]
fn a_section_offset_follows_the_parameter_it_names() {
    let mut document = pair().document;
    let id = new_drawing(&mut document);
    let cut = offset_parameter(&mut document, "5 mm");
    cut_with(&mut document, id, cut);
    let place = |document: &Document| {
        let values = ParameterValues::evaluate(document);
        document.drawing(id).unwrap().sections[0]
            .place
            .resolve(&values)
    };

    let before = place(&document);
    let edit = Transaction::single(
        "Move",
        Edit::SetParameterExpression {
            id: cut,
            expression: document.parse("1.5 cm").unwrap(),
        },
    );
    document.apply(edit).unwrap();
    let after = place(&document);

    assert_eq!(before, Ok(ResolvedPlace::Offset(5.0)));
    assert_eq!(after, Ok(ResolvedPlace::Offset(15.0)));
}

#[test]
fn a_parameter_a_section_offset_names_is_in_use_and_inlines_into_it() {
    let mut document = pair().document;
    let id = new_drawing(&mut document);
    let cut = offset_parameter(&mut document, "5 mm");
    cut_with(&mut document, id, cut);

    let users = document.parameter_user_ids(cut);
    let refused = document.can_remove_parameter(cut);
    let inlining = document.inline_parameter(cut).unwrap();
    document.apply(inlining).unwrap();

    assert_eq!(users, vec![ParameterUser::Drawing(id)]);
    assert_eq!(document.parameter_users(cut), Vec::<String>::new());
    assert!(matches!(
        refused.unwrap_err(),
        EditError::ParameterInUse { .. }
    ));
    assert!(document.parameter(cut).is_none());
    let values = ParameterValues::evaluate(&document);
    assert_eq!(
        document.drawing(id).unwrap().sections[0]
            .place
            .resolve(&values),
        Ok(ResolvedPlace::Offset(5.0))
    );
}

#[test]
fn an_offset_that_cannot_be_worked_out_is_a_typed_error() {
    let mut document = pair().document;
    let id = new_drawing(&mut document);
    let cut = offset_parameter(&mut document, "5 mm");
    cut_with(&mut document, id, cut);
    let broken = Transaction::single(
        "Break",
        Edit::SetParameterExpression {
            id: cut,
            expression: document.parse("1 mm / 0").unwrap(),
        },
    );
    document.apply(broken).unwrap();
    let far = SectionPlace::offset(1e300);

    let values = ParameterValues::evaluate(&document);
    let place = &document.drawing(id).unwrap().sections[0].place;

    assert!(matches!(
        place.resolve(&values),
        Err(DrawingViewError::Offset(_))
    ));
    assert_eq!(
        far.resolve(&values),
        Err(DrawingViewError::OffsetOutOfRange)
    );
}

#[test]
fn scaling_the_model_moves_a_section_offset_with_it() {
    let mut document = pair().document;
    let id = new_drawing(&mut document);
    let change = document
        .changing_drawing(id, "Section", |drawing| {
            drawing.sections = vec![section('A', SectionPlane::Yz, SectionPlace::offset(10.0))];
        })
        .unwrap();
    document.apply(change).unwrap();

    let scaled = document
        .scaled(&ModelScale {
            factor: 2.0,
            centre: Point3::new(1.0, 0.0, 0.0),
            values: ScaledValues::Plain,
        })
        .unwrap();
    document.apply(scaled.transaction).unwrap();
    let values = ParameterValues::evaluate(&document);

    assert_eq!(
        document.drawing(id).unwrap().sections[0]
            .place
            .resolve(&values),
        Ok(ResolvedPlace::Offset(19.0))
    );
}

fn detail(letter: char, parent: DetailParent, centre: Point2, radius: f64) -> DetailView {
    DetailView {
        letter,
        parent,
        centre,
        radius,
        scale: DetailScale::Enlarged,
    }
}

fn front_detail(letter: char, centre: Point2, radius: f64) -> DetailView {
    detail(
        letter,
        DetailParent::Standard(StandardView::Front),
        centre,
        radius,
    )
}

fn plate_front(pair: &Pair) -> ViewLines {
    let mut drawing = Drawing::new(DrawingId::from_raw(0), "Drawing 1".into());
    drawing.bodies = vec![pair.plate];
    view_lines(pair, StandardView::Front, &drawing)
}

fn endpoints(lines: &ViewLines) -> Vec<Point2> {
    lines
        .drawn_pieces(false)
        .flat_map(|piece| {
            [
                piece.curve.point(piece.range.start()),
                piece.curve.point(piece.range.end()),
            ]
        })
        .collect()
}

#[test]
fn a_detail_view_is_added_undoably_with_a_letter_shared_with_the_sections() {
    let pair = pair();
    let mut editor = Editor::new(pair.document.clone());
    let id = {
        let (transaction, id) = editor.document().adding_drawing(Vec::new()).unwrap();
        editor.apply(transaction).unwrap();
        id
    };
    let parent = DetailParent::Standard(StandardView::Front);

    let (section, a) = editor.document().adding_section_view(id).unwrap();
    editor.apply(section).unwrap();
    let (first, b) = editor
        .document()
        .adding_detail_view(id, parent, Point2::new(3.0, 1.0), 2.0)
        .unwrap();
    editor.apply(first).unwrap();
    let (next_section, c) = editor.document().adding_section_view(id).unwrap();
    editor.apply(next_section).unwrap();
    let details = editor.document().drawing(id).unwrap().details.clone();
    editor.undo().unwrap();
    editor.undo().unwrap();

    assert_eq!((a, b, c), ('A', 'B', 'C'));
    assert_eq!(details.len(), 1);
    assert_eq!(details[0].parent, parent);
    assert_eq!(details[0].centre, Point2::new(3.0, 1.0));
    assert_eq!(details[0].radius, 2.0);
    assert_eq!(details[0].scale, DetailScale::Enlarged);
    assert!(editor.document().drawing(id).unwrap().details.is_empty());
    assert_eq!(editor.redo_label(), Some("New detail view"));
}

#[test]
fn detail_views_are_checked() {
    let mut document = pair().document;
    let id = new_drawing(&mut document);
    let with = |details: Vec<DetailView>| {
        document.changing_drawing(id, "Detail", |drawing| drawing.details = details)
    };
    let centre = Point2::new(1.0, 1.0);
    let missing_parent = front_detail('B', centre, 1.0);
    let missing_parent = DetailView {
        parent: DetailParent::Section('A'),
        ..missing_parent
    };
    let sections_and_details = document.changing_drawing(id, "Detail", |drawing| {
        drawing.sections = vec![section('A', SectionPlane::Yz, SectionPlace::Centre)];
        drawing.details = vec![front_detail('A', centre, 1.0)];
    });
    let too_many = document.changing_drawing(id, "Detail", |drawing| {
        drawing.details = (0..=MAX_DETAIL_VIEWS)
            .map(|_| front_detail('A', centre, 1.0))
            .collect();
    });

    assert_eq!(
        with(vec![missing_parent]).unwrap_err(),
        EditError::DetailParentMissing {
            letter: 'B',
            parent: 'A'
        }
    );
    assert_eq!(
        with(vec![front_detail('B', centre, 0.0)]).unwrap_err(),
        EditError::DetailOutOfRange { letter: 'B' }
    );
    assert_eq!(
        with(vec![front_detail('B', Point2::new(f64::NAN, 0.0), 1.0)]).unwrap_err(),
        EditError::DetailOutOfRange { letter: 'B' }
    );
    assert_eq!(
        with(vec![DetailView {
            scale: DetailScale::Ratio(ScaleRatio::new(0, 1)),
            ..front_detail('B', centre, 1.0)
        }])
        .unwrap_err(),
        EditError::DetailScaleOutOfRange {
            letter: 'B',
            sheet: 0,
            model: 1
        }
    );
    assert_eq!(
        with(vec![front_detail('O', centre, 1.0)]).unwrap_err(),
        EditError::ViewLetterUnusable('O')
    );
    assert_eq!(
        with(vec![
            front_detail('B', centre, 1.0),
            front_detail('B', centre, 2.0)
        ])
        .unwrap_err(),
        EditError::ViewLetterRepeated('B')
    );
    assert_eq!(
        sections_and_details.unwrap_err(),
        EditError::ViewLetterRepeated('A')
    );
    assert!(matches!(
        too_many.unwrap_err(),
        EditError::DrawingTooManyDetails { .. }
    ));
    assert!(with(vec![front_detail('B', centre, 1.0)]).is_ok());
}

#[test]
fn a_detail_scale_defaults_to_twice_the_sheet_scale_as_a_preferred_ratio() {
    assert_eq!(ScaleRatio::enlarging(1.0), ScaleRatio::new(2, 1));
    assert_eq!(ScaleRatio::enlarging(0.5), ScaleRatio::new(1, 1));
    assert_eq!(ScaleRatio::enlarging(0.2), ScaleRatio::new(1, 2));
    assert_eq!(ScaleRatio::enlarging(0.3), ScaleRatio::new(1, 1));
    assert_eq!(ScaleRatio::enlarging(2.0), ScaleRatio::new(5, 1));
    assert_eq!(ScaleRatio::enlarging(40.0), ScaleRatio::new(50, 1));
    assert_eq!(DetailScale::Enlarged.ratio_at(1.0), ScaleRatio::new(2, 1));
    assert_eq!(
        DetailScale::Ratio(ScaleRatio::new(10, 1)).ratio_at(1.0),
        ScaleRatio::new(10, 1)
    );
}

#[test]
fn a_detail_keeps_exactly_the_parent_lines_inside_its_circle() {
    let pair = pair();
    let parent = plate_front(&pair);
    let corner = front_detail('B', Point2::new(0.0, 0.0), 3.0);

    let lines = detail_view(&corner, &parent).unwrap();

    assert_eq!(lines.view, DrawnView::Detail('B'));
    assert_eq!(lines.drawn_pieces(false).count(), 2);
    for piece in lines.drawn_pieces(false) {
        assert!(close(piece.length(), 3.0));
    }
    let ends = endpoints(&lines);
    assert_eq!(
        ends.iter().filter(|end| close(end.length(), 3.0)).count(),
        2
    );
    assert_eq!(ends.iter().filter(|end| end.length() < 1e-6).count(), 2);
    let bounds = lines.bounds.unwrap();
    assert!(close(bounds.size().x, 6.0) && close(bounds.size().y, 6.0));
    assert!(close(bounds.center().x, 0.0) && close(bounds.center().y, 0.0));
    assert_eq!(lines.clip.unwrap().radius, 3.0);
}

#[test]
fn a_detail_keeps_a_whole_line_inside_and_nothing_outside() {
    let pair = pair();
    let parent = plate_front(&pair);

    let around = detail_view(&front_detail('B', Point2::new(10.0, 2.0), 30.0), &parent).unwrap();
    let away = detail_view(&front_detail('C', Point2::new(100.0, 100.0), 5.0), &parent).unwrap();
    let inside_a_face =
        detail_view(&front_detail('D', Point2::new(10.0, 2.0), 1.0), &parent).unwrap();

    assert_eq!(around.pieces, parent.pieces);
    assert!(away.pieces.is_empty());
    assert!(inside_a_face.pieces.is_empty());
}

#[test]
fn a_detail_clips_an_arc_at_the_circle_exactly() {
    let pair = pair();
    let parent = plate_front(&pair);
    let template = parent.pieces[0].clone();
    let ring = caditor_kernel::Circle2::new(Point2::new(10.0, 10.0), 5.0).unwrap();
    let arcs = ViewLines {
        pieces: vec![ViewPiece {
            curve: Curve2::Circle(ring),
            range: caditor_kernel::Interval::FULL_TURN,
            visibility: Visibility::Visible,
            ..template
        }],
        ..parent
    };
    let disc = Disc {
        centre: Point2::new(15.0, 10.0),
        radius: 2.0,
    };

    let lines = detail_view(&front_detail('B', disc.centre, disc.radius), &arcs).unwrap();
    let whole = detail_view(&front_detail('C', Point2::new(10.0, 10.0), 6.0), &arcs).unwrap();
    let apart = detail_view(&front_detail('D', Point2::new(10.0, 10.0), 2.0), &arcs).unwrap();

    let seam = Point2::new(15.0, 10.0);
    assert_eq!(lines.pieces.len(), 2);
    for end in endpoints(&lines) {
        let on_circle = close((end - disc.centre).length(), 2.0);
        assert!(on_circle || (end - seam).length() < 1e-9);
    }
    for piece in &lines.pieces {
        for step in 0..=20 {
            let at = piece.range.at(f64::from(step) / 20.0);
            assert!((piece.curve.point(at) - disc.centre).length() <= 2.0 + 1e-9);
        }
    }
    let kept: f64 = lines.pieces.iter().map(|piece| piece.range.length()).sum();
    let half_angle = 0.92_f64.acos();
    assert!(close(kept, 2.0 * half_angle));
    assert_eq!(whole.pieces.len(), 1);
    assert!(close(
        whole.pieces[0].range.length(),
        caditor_kernel::Interval::FULL_TURN.length()
    ));
    assert!(apart.pieces.is_empty());
}

#[test]
fn a_detail_of_a_section_keeps_its_hatching_inside_the_circle() {
    let pair = pair();
    let cut = section_lines(
        &pair,
        &section('A', SectionPlane::Yz, SectionPlace::offset(5.0)),
    )
    .unwrap();
    let corner = DetailView {
        parent: DetailParent::Section('A'),
        ..front_detail('B', cut.bounds.unwrap().min(), 2.0)
    };

    let lines = detail_view(&corner, &cut).unwrap();
    let placed = PlacedView {
        view: DrawnView::Detail('B'),
        origin: Point2::new(100.0, 100.0),
        scale: 4.0,
    };
    let disc = lines.clip.unwrap().scaled_onto(&placed);
    let whole = hatch_lines(&lines.hatching()[0], &placed);
    let clipped = drawn_hatching(&lines, &placed);

    assert!(!lines.hatching().is_empty());
    assert!(lines.section.is_some());
    assert!(!clipped.is_empty());
    assert!(clipped.len() <= whole.len());
    for (from, to) in clipped {
        assert!((from - disc.centre).length() <= disc.radius + 1e-9);
        assert!((to - disc.centre).length() <= disc.radius + 1e-9);
    }
    assert_eq!(
        drawn_hatching(&cut, &placed).len(),
        hatch_lines(&cut.hatching()[0], &placed).len()
    );
}

fn extents_with_detail(radius: f64) -> BTreeMap<DrawnView, Aabb2> {
    let mut extents = extents_of_a_box(100.0, 60.0, 40.0);
    extents.insert(
        DrawnView::Detail('A'),
        Aabb2::from_point(Point2::new(10.0, 10.0) - Point2::splat(radius))
            .including(Point2::new(10.0, 10.0) + Point2::splat(radius)),
    );
    extents
}

#[test]
fn detail_views_stand_in_the_side_column_labelled_with_their_scale_and_circled_on_their_parent() {
    let mut drawing = Drawing::new(DrawingId::from_raw(0), "Drawing 1".into());
    drawing.details = vec![front_detail('A', Point2::new(10.0, 10.0), 8.0)];
    let extents = extents_with_detail(8.0);

    let layout = lay_out_sheet(
        &drawing,
        &TitleFields::default(),
        &extents,
        &BTreeMap::new(),
    );
    let front = placed(&layout, StandardView::Front);
    let enlarged = placed_as(&layout, DrawnView::Detail('A'));
    let isometric = placed(&layout, StandardView::Isometric);
    let circles: Vec<(Point2, f64)> = layout
        .marks
        .iter()
        .filter_map(|mark| match mark {
            SheetMark::Circle { center, radius } => Some((*center, *radius)),
            SheetMark::Line(..) => None,
        })
        .collect();
    let contents: Vec<&str> = layout
        .texts
        .iter()
        .map(|text| text.content.as_str())
        .collect();

    assert!(layout.fits);
    assert!(close(enlarged.scale, 2.0 * front.scale));
    assert!(enlarged.origin.x > isometric.origin.x);
    assert!(
        layout
            .view_area
            .contains(enlarged.on_sheet(Point2::new(2.0, 2.0)))
    );
    assert!(
        layout
            .view_area
            .contains(enlarged.on_sheet(Point2::new(18.0, 18.0)))
    );
    assert!(contents.contains(&"A (2:1)"));
    assert!(contents.contains(&"A"));
    assert!(circles.iter().any(|(centre, radius)| {
        (*centre - front.on_sheet(Point2::new(10.0, 10.0))).length() < 1e-6
            && close(*radius, 8.0 * front.scale)
    }));
    assert!(circles.iter().any(|(centre, radius)| {
        (*centre - enlarged.on_sheet(Point2::new(10.0, 10.0))).length() < 1e-6
            && close(*radius, 8.0 * enlarged.scale)
    }));
}

#[test]
fn a_chosen_detail_scale_is_kept_and_labelled_and_fitting_leaves_room_for_it() {
    let mut drawing = Drawing::new(DrawingId::from_raw(0), "Drawing 1".into());
    drawing.details = vec![DetailView {
        scale: DetailScale::Ratio(ScaleRatio::new(10, 1)),
        ..front_detail('A', Point2::new(10.0, 10.0), 8.0)
    }];
    let extents = extents_with_detail(8.0);

    let layout = lay_out_sheet(
        &drawing,
        &TitleFields::default(),
        &extents,
        &BTreeMap::new(),
    );
    let enlarged = placed_as(&layout, DrawnView::Detail('A'));

    assert!(close(enlarged.scale, 10.0));
    assert!(layout.texts.iter().any(|text| text.content == "A (10:1)"));
    assert!(layout.fits);
    assert!(
        layout
            .view_area
            .contains(enlarged.on_sheet(Point2::new(18.0, 18.0)))
    );
}

#[test]
fn deleting_the_section_a_detail_enlarges_is_refused_and_scaling_the_model_moves_a_circle() {
    let mut document = pair().document;
    let id = new_drawing(&mut document);
    let change = document
        .changing_drawing(id, "Detail", |drawing| {
            drawing.sections = vec![section('A', SectionPlane::Yz, SectionPlace::Centre)];
            drawing.details = vec![
                front_detail('B', Point2::new(3.0, 4.0), 2.0),
                detail('C', DetailParent::Section('A'), Point2::new(1.0, 2.0), 1.0),
            ];
        })
        .unwrap();
    document.apply(change).unwrap();

    let refused = document.changing_drawing(id, "Delete", |drawing| drawing.sections.clear());
    let scaled = document
        .scaled(&ModelScale {
            factor: 2.0,
            centre: Point3::new(1.0, 0.0, 0.0),
            values: ScaledValues::Plain,
        })
        .unwrap();
    document.apply(scaled.transaction).unwrap();
    let details = &document.drawing(id).unwrap().details;

    assert_eq!(
        refused.unwrap_err(),
        EditError::DetailParentMissing {
            letter: 'C',
            parent: 'A'
        }
    );
    assert_eq!(details[0].centre, Point2::new(5.0, 8.0));
    assert_eq!(details[0].radius, 4.0);
    assert_eq!(details[1].radius, 2.0);
    assert_eq!(details[1].centre, Point2::new(2.0, 4.0));
}
