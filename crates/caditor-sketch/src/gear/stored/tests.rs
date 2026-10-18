use std::collections::BTreeSet;

use caditor_expression::{EvalError, Expression, ParameterId, Quantity, Unit};
use caditor_geometry::{Plane, Point2, Vector2};

use super::{BORE_TOOTH, Gear, GearValueError, gear_curve};
use crate::{Constraint, EntityId, GearError, Sketch, SketchError, SpurGear};

const TEETH: ParameterId = ParameterId::from_raw(7);

fn spur_gear(centre: EntityId) -> Gear {
    Gear {
        centre,
        module: Expression::measure(2.0, Unit::Millimetre),
        teeth: Expression::number(18.0),
        pressure_angle: Expression::measure(20.0, Unit::Degree),
        profile_shift: Expression::number(0.0),
        root_fillet: Expression::measure(0.5, Unit::Millimetre),
        bore: Expression::measure(6.0, Unit::Millimetre),
        turn: Expression::measure(0.0, Unit::Degree),
    }
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
fn a_gear_stands_on_a_point_that_cannot_go_while_the_gear_stays() {
    let mut sketch = Sketch::new(Plane::XY);
    let centre = sketch.add_point(Point2::new(3.0, 4.0));

    let gear = sketch.add_parametric_gear(spur_gear(centre)).unwrap();
    let refused = sketch.remove_unused_entity(centre);

    assert!(matches!(refused, Err(SketchError::InUse { .. })));
    assert_eq!(sketch.gears_using(centre), vec![gear]);
    assert!(sketch.free_points().is_empty());
    assert!(sketch.layout_points().is_empty());
    assert_eq!(sketch.entity_label(gear), format!("Gear {gear}"));

    sketch.remove_gear(gear).unwrap();
    assert!(sketch.remove_unused_entity(centre).is_ok());
}

#[test]
fn a_gear_needs_a_point_to_stand_on() {
    let mut sketch = Sketch::new(Plane::XY);
    let line = sketch.add_line(Point2::ZERO, Point2::X);

    let on_line = sketch.add_parametric_gear(spur_gear(line));
    let missing = sketch.add_parametric_gear(spur_gear(EntityId::from_raw(99)));

    assert_eq!(on_line, Err(SketchError::NotAPoint(line)));
    assert_eq!(
        missing,
        Err(SketchError::MissingEntity(EntityId::from_raw(99)))
    );
    assert_eq!(sketch.gears().len(), 0);
}

#[test]
fn gears_share_the_sketch_numbering_and_keep_their_centre_when_edited() {
    let mut sketch = Sketch::new(Plane::XY);
    let centre = sketch.add_point(Point2::ZERO);
    let other = sketch.add_point(Point2::X);
    let gear = sketch.add_parametric_gear(spur_gear(centre)).unwrap();

    let duplicate = sketch.insert_gear(other, spur_gear(centre));
    let moved = sketch.replace_gear(gear, spur_gear(other));
    let renumbered = sketch.replace_gear(
        gear,
        Gear {
            teeth: Expression::number(30.0),
            ..spur_gear(centre)
        },
    );
    let after = sketch.add_point(Point2::Y);

    assert_eq!(duplicate, Err(SketchError::DuplicateId(other.raw())));
    assert!(matches!(moved, Err(SketchError::ChangesStructure { .. })));
    assert_eq!(renumbered.unwrap().teeth, Expression::number(18.0));
    assert_eq!(sketch.gear(gear).unwrap().teeth, Expression::number(30.0));
    assert!(after.raw() > gear.raw());
}

#[test]
fn solving_works_out_the_values_from_parameters_and_the_centre_from_its_constraints() {
    let mut sketch = Sketch::new(Plane::XY);
    let centre = sketch.add_point(Point2::new(1.0, 2.0));
    sketch
        .add_constraint(Constraint::Fix {
            point: centre,
            at: Point2::new(10.0, 5.0),
        })
        .unwrap();
    let gear = sketch
        .add_parametric_gear(Gear {
            teeth: Expression::Parameter(TEETH),
            turn: Expression::measure(15.0, Unit::Degree),
            ..spur_gear(centre)
        })
        .unwrap();

    let solved = sketch.solve(&teeth_of(20.0), &|| false).unwrap().geometry;
    let values = solved.gear_values(gear).unwrap();
    let outline = solved.gear_outline(gear).unwrap();

    assert_eq!(values.spur.teeth, 20);
    assert_eq!(values.turn, 15.0);
    assert_eq!(outline.centre, Point2::new(10.0, 5.0));
    assert_eq!(outline.teeth, 20);
    assert_eq!(sketch.gear_values(gear), None);
    assert!(sketch.uses_parameter(TEETH));
    assert!(sketch.parameters().contains(&TEETH));
}

#[test]
fn values_that_cannot_make_a_gear_are_refused_at_solve_in_words() {
    let mut sketch = Sketch::new(Plane::XY);
    let centre = sketch.add_point(Point2::ZERO);
    sketch
        .add_parametric_gear(Gear {
            teeth: Expression::Parameter(TEETH),
            ..spur_gear(centre)
        })
        .unwrap();

    let missing = sketch.solve(&no_parameters, &|| false);
    let fractional = sketch.solve(&teeth_of(12.5), &|| false);
    let few = sketch.solve(&teeth_of(2.0), &|| false);
    let undercut = sketch.solve(&teeth_of(6.0), &|| false);

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
        }) if matches!(*error, GearError::TeethOutOfRange { .. })
    ));
    assert!(matches!(
        undercut,
        Err(SketchError::Gear {
            reason: GearValueError::Build(error),
            ..
        }) if matches!(*error, GearError::Undercut { .. })
    ));
}

#[test]
fn changing_a_gear_changes_the_geometry() {
    let mut sketch = Sketch::new(Plane::XY);
    let centre = sketch.add_point(Point2::ZERO);
    let gear = sketch.add_parametric_gear(spur_gear(centre)).unwrap();
    let before = sketch.clone();

    sketch
        .replace_gear(
            gear,
            Gear {
                turn: Expression::measure(5.0, Unit::Degree),
                ..spur_gear(centre)
            },
        )
        .unwrap();

    assert!(!before.same_geometry(&sketch));
    assert!(before.heap_size() > 0);
}

#[test]
fn curve_names_belong_to_their_gear_and_never_to_a_text_or_a_stored_entity() {
    let mut sketch = Sketch::new(Plane::XY);
    let centre = sketch.add_point(Point2::ZERO);
    let gear = sketch.add_parametric_gear(spur_gear(centre)).unwrap();

    let flank = gear_curve(gear, 3, 1).unwrap();
    let bore = gear_curve(gear, BORE_TOOTH, 0).unwrap();
    let elsewhere = gear_curve(centre, 3, 1).unwrap();

    assert_ne!(flank, bore);
    assert_eq!(sketch.gear_of_curve(flank), Some(gear));
    assert_eq!(sketch.gear_of_curve(bore), Some(gear));
    assert_eq!(sketch.gear_of_curve(elsewhere), None);
    assert_eq!(sketch.gear_of_curve(gear.raw()), None);
    assert_eq!(sketch.text_of_letter_curve(flank), None);
    assert!(!EntityId::from_raw(flank).is_reference());
    assert_eq!(
        sketch.entity_label(EntityId::from_raw(flank)),
        format!("Gear {gear}")
    );
    assert_eq!(gear_curve(gear, 1 << 11, 0), None);
    assert_eq!(gear_curve(gear, 0, 1 << 12), None);
}

#[test]
fn more_teeth_add_named_curves_and_fewer_remove_them_leaving_the_rest_named_alike() {
    let spur = |teeth: u32| SpurGear {
        module: 2.0,
        teeth,
        pressure_angle: 20.0,
        profile_shift: 0.0,
        root_fillet: 0.5,
        bore: 0.0,
    };
    let names = |teeth: u32| -> BTreeSet<(usize, usize)> {
        spur(teeth)
            .outline(Point2::ZERO)
            .unwrap()
            .named_pieces()
            .into_iter()
            .map(|(name, _)| (name.tooth, name.segment))
            .collect()
    };

    let eighteen = names(18);
    let twenty = names(20);

    assert_eq!(eighteen.len(), 18 * 8);
    assert_eq!(twenty.len(), 20 * 8);
    assert!(eighteen.is_subset(&twenty));
    assert_eq!(
        twenty
            .difference(&eighteen)
            .map(|name| name.0)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([18, 19])
    );
}

#[test]
fn a_sharp_rooted_gear_names_every_piece_once() {
    let sharp = SpurGear {
        module: 2.0,
        teeth: 20,
        pressure_angle: 20.0,
        profile_shift: 0.0,
        root_fillet: 0.0,
        bore: 0.0,
    };
    let outline = sharp.outline(Point2::ZERO).unwrap();

    let named = outline.named_pieces();
    let unique: BTreeSet<_> = named
        .iter()
        .map(|(name, _)| (name.tooth, name.segment))
        .collect();

    assert_eq!(unique.len(), named.len());
    assert_eq!(named.len(), outline.pieces.len());
    assert!(named.iter().all(|(name, _)| name.tooth < 20));
}

#[test]
fn a_copied_gear_is_pasted_on_a_copy_of_its_centre() {
    let mut sketch = Sketch::new(Plane::XY);
    let centre = sketch.add_point(Point2::new(1.0, 2.0));
    let gear = sketch.add_parametric_gear(spur_gear(centre)).unwrap();

    let clip = sketch.clip(&[gear]).unwrap();
    let pasted = sketch.paste(&clip, Vector2::new(10.0, 0.0)).unwrap();
    let copies: Vec<EntityId> = pasted
        .iter()
        .copied()
        .filter(|id| sketch.gear(*id).is_some())
        .collect();
    let copy = sketch.gear(copies[0]).unwrap();

    assert_eq!(clip.gear_count(), 1);
    assert_eq!(clip.lone_point_count(), 0);
    assert_eq!(copies.len(), 1);
    assert_eq!(copy.teeth, Expression::number(18.0));
    assert_ne!(copy.centre, centre);
    assert_eq!(sketch.point(copy.centre), Some(Point2::new(11.0, 2.0)));
}
