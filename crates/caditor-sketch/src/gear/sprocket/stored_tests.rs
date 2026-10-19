use std::collections::BTreeSet;

use caditor_expression::{EvalError, Expression, ParameterId, Quantity, Unit};
use caditor_geometry::{Plane, Point2, Vector2};

use super::{ChainSize, MIN_SPROCKET_TEETH, Sprocket};
use crate::{
    Constraint, EntityId, Gear, GearError, GearValueError, Sketch, SketchError, ToothedOutline,
    gear_curve,
};

const TEETH: ParameterId = ParameterId::from_raw(7);

fn chain() -> ChainSize {
    ChainSize::find("08B").unwrap()
}

fn stored(centre: EntityId, teeth: f64) -> Gear {
    Gear::sprocket(
        centre,
        chain(),
        Expression::number(teeth),
        Expression::measure(0.0, Unit::Millimetre),
        Expression::measure(0.0, Unit::Degree),
    )
}

fn no_parameters(_: ParameterId) -> Result<Quantity, EvalError> {
    Err(EvalError::ParameterMissing)
}

fn teeth_of(count: f64) -> impl Fn(ParameterId) -> Result<Quantity, EvalError> {
    move |id| {
        if id == TEETH {
            Ok(Quantity::plain(count))
        } else {
            Err(EvalError::ParameterMissing)
        }
    }
}

#[test]
fn a_stored_sprocket_stands_on_a_point_and_lists_only_its_own_values() {
    let mut sketch = Sketch::new(Plane::XY);
    let centre = sketch.add_point(Point2::new(3.0, 4.0));

    let id = sketch.add_parametric_gear(stored(centre, 17.0)).unwrap();
    let refused = sketch.remove_unused_entity(centre);
    let gear = sketch.gear(id).unwrap();

    assert!(matches!(refused, Err(SketchError::InUse { .. })));
    assert!(gear.is_sprocket());
    assert_eq!(gear.chain, Some(chain()));
    assert_eq!(gear.expressions().len(), 3);
    assert!(sketch.free_points().is_empty());
    assert_eq!(sketch.entity_label(id), format!("Sprocket {id}"));

    sketch.remove_gear(id).unwrap();
    assert!(sketch.remove_unused_entity(centre).is_ok());
}

#[test]
fn solving_works_out_a_sprocket_from_its_parameters_and_the_centre_from_its_constraints() {
    let mut sketch = Sketch::new(Plane::XY);
    let centre = sketch.add_point(Point2::new(1.0, 2.0));
    sketch
        .add_constraint(Constraint::Fix {
            point: centre,
            at: Point2::new(10.0, 5.0),
        })
        .unwrap();
    let id = sketch
        .add_parametric_gear(Gear {
            teeth: Expression::Parameter(TEETH),
            turn: Expression::measure(15.0, Unit::Degree),
            ..stored(centre, 17.0)
        })
        .unwrap();

    let solved = sketch.solve(&teeth_of(19.0), &|| false).unwrap().geometry;
    let values = solved.gear_values(id).unwrap();
    let outline = solved.toothed_outline(id).unwrap();

    assert_eq!(values.shape.sprocket().map(|s| s.teeth), Some(19));
    assert_eq!(values.shape.spur(), None);
    assert_eq!(values.turn, 15.0);
    assert_eq!(outline.centre(), Point2::new(10.0, 5.0));
    assert_eq!(outline.pieces().len(), 19 * 4);
    assert!(solved.gear_outline(id).is_none());
    assert!(sketch.uses_parameter(TEETH));
    assert!(sketch.parameters().contains(&TEETH));
}

#[test]
fn the_turn_carries_the_first_gap_about_the_centre() {
    let mut sketch = Sketch::new(Plane::XY);
    let centre = sketch.add_point(Point2::ZERO);
    let id = sketch
        .add_parametric_gear(Gear {
            turn: Expression::measure(90.0, Unit::Degree),
            ..stored(centre, 20.0)
        })
        .unwrap();

    let solved = sketch.solve(&no_parameters, &|| false).unwrap().geometry;
    let turned = solved.toothed_outline(id).unwrap();
    let plain = Sprocket {
        chain: chain(),
        teeth: 20,
        bore: 0.0,
    }
    .outline(Point2::ZERO)
    .unwrap();
    let seat = |pieces: &[crate::GearPiece]| pieces.get(1).map(crate::GearPiece::from);

    let from = seat(turned.pieces()).unwrap();
    let was = seat(&plain.pieces).unwrap();

    assert!((from.x + was.y).abs() < 1e-9);
    assert!((from.y - was.x).abs() < 1e-9);
}

#[test]
fn a_bore_is_drawn_and_named_apart_from_the_gaps() {
    let mut sketch = Sketch::new(Plane::XY);
    let centre = sketch.add_point(Point2::ZERO);
    let id = sketch
        .add_parametric_gear(Gear {
            bore: Expression::measure(20.0, Unit::Millimetre),
            ..stored(centre, 25.0)
        })
        .unwrap();

    let solved = sketch.solve(&no_parameters, &|| false).unwrap().geometry;
    let outline = solved.toothed_outline(id).unwrap();

    assert_eq!(outline.bore(), Some(10.0));
    assert_eq!(outline.radii().len(), 3);
    assert!(matches!(outline, ToothedOutline::Sprocket(_)));
}

#[test]
fn values_that_cannot_make_a_sprocket_are_refused_at_solve_in_words() {
    let mut sketch = Sketch::new(Plane::XY);
    let centre = sketch.add_point(Point2::ZERO);
    sketch
        .add_parametric_gear(Gear {
            teeth: Expression::Parameter(TEETH),
            ..stored(centre, 17.0)
        })
        .unwrap();

    let missing = sketch.solve(&no_parameters, &|| false);
    let fractional = sketch.solve(&teeth_of(12.5), &|| false);
    let few = sketch.solve(&teeth_of(f64::from(MIN_SPROCKET_TEETH - 1)), &|| false);
    let many = sketch.solve(&teeth_of(400.0), &|| false);

    assert!(matches!(
        missing,
        Err(SketchError::Gear {
            reason: GearValueError::Teeth(_),
            ..
        })
    ));
    assert!(matches!(
        fractional,
        Err(SketchError::Gear {
            reason: GearValueError::TeethNotWhole { .. },
            ..
        })
    ));
    assert!(matches!(
        few,
        Err(SketchError::Gear {
            reason: GearValueError::Build(error),
            ..
        }) if matches!(*error, GearError::SprocketTeethOutOfRange { .. })
    ));
    assert!(matches!(
        many,
        Err(SketchError::Gear {
            reason: GearValueError::Build(error),
            ..
        }) if matches!(*error, GearError::SprocketTeethOutOfRange { .. })
    ));
}

#[test]
fn a_bore_larger_than_the_roots_allow_is_refused_in_words() {
    let mut sketch = Sketch::new(Plane::XY);
    let centre = sketch.add_point(Point2::ZERO);
    sketch
        .add_parametric_gear(Gear {
            bore: Expression::measure(500.0, Unit::Millimetre),
            ..stored(centre, 17.0)
        })
        .unwrap();

    let refused = sketch.solve(&no_parameters, &|| false);

    assert!(matches!(
        refused,
        Err(SketchError::Gear {
            reason: GearValueError::Build(error),
            ..
        }) if matches!(*error, GearError::BoreTooLarge { .. })
    ));
}

#[test]
fn the_chain_can_change_but_a_sprocket_never_becomes_a_spur_gear() {
    let mut sketch = Sketch::new(Plane::XY);
    let centre = sketch.add_point(Point2::ZERO);
    let id = sketch.add_parametric_gear(stored(centre, 17.0)).unwrap();
    let wider = ChainSize::find("12B").unwrap();

    let changed = sketch.replace_gear(
        id,
        Gear {
            chain: Some(wider),
            ..stored(centre, 17.0)
        },
    );
    let spur = sketch.replace_gear(
        id,
        Gear {
            chain: None,
            ..stored(centre, 17.0)
        },
    );

    assert!(changed.is_ok());
    assert!(matches!(spur, Err(SketchError::ChangesStructure { .. })));
    assert_eq!(sketch.gear(id).unwrap().chain, Some(wider));
}

#[test]
fn a_sprocket_cannot_be_the_gear_another_meshes_with() {
    let mut sketch = Sketch::new(Plane::XY);
    let centre = sketch.add_point(Point2::ZERO);
    let other = sketch.add_point(Point2::new(50.0, 0.0));
    let id = sketch.add_parametric_gear(stored(centre, 17.0)).unwrap();

    let partner = sketch.add_parametric_gear(Gear::meshed(
        other,
        id,
        Expression::number(20.0),
        Expression::measure(0.0, Unit::Millimetre),
        Expression::measure(0.0, Unit::Degree),
    ));

    assert!(matches!(partner, Err(SketchError::WrongKind { .. })));
}

#[test]
fn changing_a_sprocket_changes_the_geometry() {
    let mut sketch = Sketch::new(Plane::XY);
    let centre = sketch.add_point(Point2::ZERO);
    let id = sketch.add_parametric_gear(stored(centre, 17.0)).unwrap();
    let before = sketch.clone();

    sketch
        .replace_gear(
            id,
            Gear {
                chain: ChainSize::find("10A"),
                ..stored(centre, 17.0)
            },
        )
        .unwrap();

    assert!(!before.same_geometry(&sketch));
}

#[test]
fn curves_are_named_by_gap_and_segment_and_more_teeth_keep_the_old_names() {
    let mut sketch = Sketch::new(Plane::XY);
    let centre = sketch.add_point(Point2::ZERO);
    let id = sketch.add_parametric_gear(stored(centre, 17.0)).unwrap();
    let names = |teeth: u32| -> BTreeSet<(usize, usize)> {
        Sprocket {
            chain: chain(),
            teeth,
            bore: 0.0,
        }
        .outline(Point2::ZERO)
        .unwrap()
        .named_pieces()
        .into_iter()
        .map(|(name, _)| (name.tooth, name.segment))
        .collect()
    };

    let seventeen = names(17);
    let nineteen = names(19);
    let seat = gear_curve(id, 3, 1).unwrap();

    assert_eq!(seventeen.len(), 17 * 4);
    assert_eq!(nineteen.len(), 19 * 4);
    assert!(seventeen.is_subset(&nineteen));
    assert_eq!(sketch.gear_of_curve(seat), Some(id));
    assert_eq!(
        sketch.entity_label(EntityId::from_raw(seat)),
        format!("Sprocket {id}")
    );
}

#[test]
fn a_copied_sprocket_is_pasted_on_a_copy_of_its_centre_with_its_chain() {
    let mut sketch = Sketch::new(Plane::XY);
    let centre = sketch.add_point(Point2::new(1.0, 2.0));
    let id = sketch.add_parametric_gear(stored(centre, 17.0)).unwrap();

    let clip = sketch.clip(&[id]).unwrap();
    let pasted = sketch.paste(&clip, Vector2::new(10.0, 0.0)).unwrap();
    let copies: Vec<EntityId> = pasted
        .iter()
        .copied()
        .filter(|id| sketch.gear(*id).is_some())
        .collect();
    let copy = sketch.gear(copies[0]).unwrap();

    assert_eq!(clip.gear_count(), 1);
    assert_eq!(copies.len(), 1);
    assert_eq!(copy.chain, Some(chain()));
    assert_eq!(copy.teeth, Expression::number(17.0));
    assert_eq!(sketch.point(copy.centre), Some(Point2::new(11.0, 2.0)));
}
