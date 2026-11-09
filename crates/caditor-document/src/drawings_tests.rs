use std::collections::BTreeMap;

use caditor_geometry::{Aabb2, Point2};
use caditor_kernel::Visibility;

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

fn extents_of_a_box(x: f64, y: f64, z: f64) -> BTreeMap<StandardView, Aabb2> {
    let rectangle = |width: f64, height: f64| {
        Aabb2::from_point(Point2::ZERO).including(Point2::new(width, height))
    };
    BTreeMap::from([
        (StandardView::Front, rectangle(x, z)),
        (StandardView::Top, rectangle(x, y)),
        (
            StandardView::Right,
            Aabb2::from_point(Point2::ZERO).including(Point2::new(y, z)),
        ),
        (
            StandardView::Left,
            Aabb2::from_point(Point2::new(-y, 0.0)).including(Point2::new(0.0, z)),
        ),
        (StandardView::Isometric, rectangle(x, z)),
    ])
}

fn placed(layout: &SheetPlan, view: StandardView) -> PlacedView {
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

    let layout = lay_out_sheet(&drawing, &TitleFields::default(), &extents);

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

    let first = lay_out_sheet(&drawing, &TitleFields::default(), &extents);
    drawing.projection = Projection::ThirdAngle;
    let third = lay_out_sheet(&drawing, &TitleFields::default(), &extents);

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

    let layout = lay_out_sheet(&drawing, &TitleFields::default(), &extents);
    drawing.size = SheetSize::A4;
    drawing.scale = DrawingScale::Fit;
    let small = lay_out_sheet(&drawing, &TitleFields::default(), &extents);

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
    let layout = lay_out_sheet(&drawing, &fields, &BTreeMap::new());
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
