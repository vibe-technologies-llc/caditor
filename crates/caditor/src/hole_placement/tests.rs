use std::collections::BTreeMap;

use caditor_document::SketchFeature;
use caditor_geometry::{Plane, Point2};
use caditor_sketch::{Constraint, Sketch};

use super::{Anchor, of_sketch};

fn definition(sketch: Sketch) -> SketchFeature {
    SketchFeature {
        sketch,
        attachment: None,
        projections: BTreeMap::new(),
    }
}

#[test]
fn loose_points_are_free_holes_in_the_order_they_were_placed() {
    let mut sketch = Sketch::new(Plane::XY);
    let first = sketch.add_point(Point2::new(5.0, 5.0));
    let second = sketch.add_point(Point2::new(15.0, 5.0));

    let placement = of_sketch(&definition(sketch)).expect("loose points are a placement");

    assert_eq!(placement.holes.len(), 2);
    assert_eq!(placement.holes[0].point, first);
    assert_eq!(placement.holes[1].point, second);
    assert!(
        placement
            .holes
            .iter()
            .all(|hole| hole.anchor == Anchor::Free)
    );
    assert_eq!(
        placement.taken(),
        vec![Point2::new(5.0, 5.0), Point2::new(15.0, 5.0)]
    );
}

#[test]
fn a_sketch_drawn_by_hand_is_no_placement() {
    let mut with_line = Sketch::new(Plane::XY);
    with_line.add_point(Point2::new(5.0, 5.0));
    with_line.add_line(Point2::new(0.0, 0.0), Point2::new(10.0, 0.0));
    let mut with_constraint = Sketch::new(Plane::XY);
    let first = with_constraint.add_point(Point2::new(5.0, 5.0));
    let second = with_constraint.add_point(Point2::new(15.0, 5.0));
    with_constraint
        .add_constraint(Constraint::HorizontalPoints(first, second))
        .unwrap();
    let empty = Sketch::new(Plane::XY);

    assert_eq!(of_sketch(&definition(with_line)), None);
    assert_eq!(of_sketch(&definition(with_constraint)), None);
    assert_eq!(of_sketch(&definition(empty)), None);
}
