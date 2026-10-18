use caditor_expression::{EvalError, Expression, ParameterId, Quantity, Unit};
use caditor_geometry::{Plane, Point2};

use crate::{Constraint, EntityId, Sketch, SketchError, Text, TextError, TextSize, letter_curve};

const HEIGHT: ParameterId = ParameterId::from_raw(4);

fn label(content: &str, anchor: EntityId) -> Text {
    Text {
        anchor,
        content: content.to_owned(),
        height: Expression::measure(5.0, Unit::Millimetre),
        angle: Expression::measure(0.0, Unit::Degree),
        bold: false,
        italic: false,
    }
}

fn no_parameters(_: ParameterId) -> Result<Quantity, EvalError> {
    Err(EvalError::ParameterMissing)
}

#[test]
fn a_text_stands_on_a_point_that_cannot_go_while_the_text_stays() {
    let mut sketch = Sketch::new(Plane::XY);
    let anchor = sketch.add_point(Point2::new(3.0, 4.0));

    let text = sketch.add_text(label("Part 7", anchor)).unwrap();
    let refused = sketch.remove_unused_entity(anchor);

    assert!(matches!(refused, Err(SketchError::InUse { .. })));
    assert_eq!(sketch.texts_using(anchor), vec![text]);
    assert!(sketch.free_points().is_empty());
    assert!(sketch.layout_points().is_empty());
    assert_eq!(sketch.entity_label(text), format!("Text {text}"));

    sketch.remove_text(text).unwrap();
    assert!(sketch.remove_unused_entity(anchor).is_ok());
}

#[test]
fn a_text_needs_a_point_and_one_line_of_letters() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::X);
    let anchor = sketch.add_point(Point2::ZERO);

    let on_line = sketch.add_text(label("A", line));
    let missing = sketch.add_text(label("A", EntityId::from_raw(99)));
    let blank = sketch.add_text(label("  ", anchor));
    let broken = sketch.add_text(label("A\nB", anchor));
    let long = sketch.add_text(label(&"x".repeat(crate::MAX_TEXT_CHARS + 1), anchor));

    assert_eq!(on_line, Err(SketchError::NotAPoint(line)));
    assert_eq!(
        missing,
        Err(SketchError::MissingEntity(EntityId::from_raw(99)))
    );
    assert_eq!(blank, Err(SketchError::InvalidText(TextError::Blank)));
    assert_eq!(
        broken,
        Err(SketchError::InvalidText(TextError::ControlCharacter))
    );
    assert!(matches!(
        long,
        Err(SketchError::InvalidText(TextError::TooLong { .. }))
    ));
    assert_eq!(sketch.texts().len(), 0);
}

#[test]
fn texts_share_the_sketch_numbering_and_keep_their_anchor_when_edited() {
    let mut sketch = Sketch::new(Plane::XY);
    let anchor = sketch.add_point(Point2::ZERO);
    let other = sketch.add_point(Point2::X);
    let text = sketch.add_text(label("A", anchor)).unwrap();

    let duplicate = sketch.insert_text(other, label("B", anchor));
    let moved = sketch.replace_text(text, label("A", other));
    let renamed = sketch.replace_text(text, label("Bolt", anchor));
    let after = sketch.add_point(Point2::Y);

    assert_eq!(duplicate, Err(SketchError::DuplicateId(other.raw())));
    assert!(matches!(moved, Err(SketchError::ChangesStructure { .. })));
    assert_eq!(renamed.unwrap().content, "A");
    assert_eq!(sketch.text(text).unwrap().content, "Bolt");
    assert!(after.raw() > text.raw());
}

#[test]
fn solving_works_out_the_height_and_angle_from_parameters() {
    let mut sketch = Sketch::new(Plane::XY);
    let anchor = sketch.add_point(Point2::new(1.0, 2.0));
    sketch
        .add_constraint(Constraint::Fix {
            point: anchor,
            at: Point2::new(1.0, 2.0),
        })
        .ok();
    let text = sketch
        .add_text(Text {
            height: Expression::Parameter(HEIGHT),
            angle: Expression::measure(30.0, Unit::Degree),
            ..label("M8", anchor)
        })
        .unwrap();

    let height = |id: ParameterId| {
        if id == HEIGHT {
            Ok(Quantity::length(7.5))
        } else {
            Err(EvalError::ParameterMissing)
        }
    };
    let solved = sketch.solve(&height, &|| false).unwrap().geometry;
    let unsolved = sketch.solve(&no_parameters, &|| false);

    assert_eq!(
        solved.text_size(text),
        Some(TextSize {
            height: 7.5,
            angle: 30.0
        })
    );
    assert_eq!(sketch.text_size(text), None);
    assert!(sketch.uses_parameter(HEIGHT));
    assert!(sketch.parameters().contains(&HEIGHT));
    assert!(matches!(
        unsolved,
        Err(SketchError::Text { text: failed, reason: TextError::Height(_), .. }) if failed == text
    ));
}

#[test]
fn a_height_not_above_zero_is_refused_at_solve() {
    let mut sketch = Sketch::new(Plane::XY);
    let anchor = sketch.add_point(Point2::ZERO);
    sketch
        .add_text(Text {
            height: Expression::measure(0.0, Unit::Millimetre),
            ..label("A", anchor)
        })
        .unwrap();

    let solved = sketch.solve(&no_parameters, &|| false);

    assert!(matches!(
        solved,
        Err(SketchError::Text {
            reason: TextError::HeightNotPositive,
            ..
        })
    ));
}

#[test]
fn changing_a_text_changes_the_geometry() {
    let mut sketch = Sketch::new(Plane::XY);
    let anchor = sketch.add_point(Point2::ZERO);
    let text = sketch.add_text(label("A", anchor)).unwrap();
    let before = sketch.clone();

    sketch
        .replace_text(
            text,
            Text {
                bold: true,
                ..label("A", anchor)
            },
        )
        .unwrap();

    assert!(!before.same_geometry(&sketch));
    assert!(before.heap_size() > 0);
}

#[test]
fn letter_curves_name_their_text_and_never_a_stored_entity() {
    let mut sketch = Sketch::new(Plane::XY);
    let anchor = sketch.add_point(Point2::ZERO);
    let text = sketch.add_text(label("Ab", anchor)).unwrap();

    let first = letter_curve(text, 0, 0).unwrap();
    let last = letter_curve(text, 1, 37).unwrap();
    let elsewhere = letter_curve(anchor, 0, 0).unwrap();

    assert_ne!(first, last);
    assert!(first >= 1 << 63);
    assert!(!EntityId::from_raw(last).is_reference());
    assert_eq!(sketch.text_of_letter_curve(first), Some(text));
    assert_eq!(sketch.text_of_letter_curve(last), Some(text));
    assert_eq!(sketch.text_of_letter_curve(elsewhere), None);
    assert_eq!(sketch.text_of_letter_curve(anchor.raw()), None);
    assert_eq!(
        sketch.entity_label(EntityId::from_raw(last)),
        format!("Text {text}")
    );
    assert_eq!(letter_curve(text, 1 << 11, 0), None);
    assert_eq!(letter_curve(text, 0, 1 << 12), None);
}
