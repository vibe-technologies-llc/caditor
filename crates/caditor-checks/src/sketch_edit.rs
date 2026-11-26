use std::collections::BTreeMap;

use arbitrary::{Result, Unstructured};
use caditor_expression::{Expression, Unit};
use caditor_geometry::Plane;
use caditor_sketch::{
    Caps, ChamferSize, Corner, Dimensioned, Entity, EntityId, Pick, Side, Sketch,
    fuzzing::{kept_uses, uses_from_scratch},
};

use crate::{
    angle, length, point,
    sketch::{self, STRANGE_LENGTHS, any_entity, points},
};

const MOST_EDITS: usize = 8;
const MOST_PICKED: usize = 4;
const MOST_SPLIT_WORK: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum EditKind {
    PointOnCurve,
    Trim,
    Extend,
    Split,
    Break,
    Offset,
    OffsetBothSides,
    Fillet,
    Chamfer,
    Join,
}

impl EditKind {
    pub const ALL: [Self; 10] = [
        Self::PointOnCurve,
        Self::Trim,
        Self::Extend,
        Self::Split,
        Self::Break,
        Self::Offset,
        Self::OffsetBothSides,
        Self::Fillet,
        Self::Chamfer,
        Self::Join,
    ];
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    pub applied: BTreeMap<EditKind, usize>,
    pub refused: BTreeMap<EditKind, usize>,
}

impl Tally {
    pub fn add(&mut self, other: &Self) {
        for (kind, count) in &other.applied {
            *self.applied.entry(*kind).or_default() += count;
        }
        for (kind, count) in &other.refused {
            *self.refused.entry(*kind).or_default() += count;
        }
    }
}

fn curves(sketch: &Sketch) -> Vec<EntityId> {
    sketch
        .entities()
        .filter(|(_, entity)| !matches!(entity, Entity::Point(_)))
        .map(|(id, _)| id)
        .collect()
}

fn pick_from(
    input: &mut Unstructured,
    preferred: &[EntityId],
    sketch: &Sketch,
) -> Result<EntityId> {
    if preferred.is_empty() || input.ratio(1u8, 8)? {
        return any_entity(input, sketch);
    }
    Ok(*input.choose(preferred)?)
}

fn curve(input: &mut Unstructured, sketch: &Sketch) -> Result<EntityId> {
    pick_from(input, &curves(sketch), sketch)
}

fn some_curves(input: &mut Unstructured, sketch: &Sketch) -> Result<Vec<EntityId>> {
    let drawn = curves(sketch);
    (0..input.int_in_range(0..=MOST_PICKED)?)
        .map(|_| pick_from(input, &drawn, sketch))
        .collect()
}

fn distance(input: &mut Unstructured) -> Result<f64> {
    if input.ratio(1u8, 16)? {
        Ok(*input.choose(&STRANGE_LENGTHS)?)
    } else {
        length(input)
    }
}

fn dimensioned_length(input: &mut Unstructured) -> Result<Dimensioned> {
    let value = distance(input)?;
    Ok(Dimensioned {
        expression: Expression::measure(value, Unit::Millimetre),
        value,
    })
}

fn dimensioned_angle(input: &mut Unstructured) -> Result<Dimensioned> {
    let value = angle(input)?.to_degrees();
    Ok(Dimensioned {
        expression: Expression::measure(value, Unit::Degree),
        value,
    })
}

fn side(input: &mut Unstructured) -> Result<Side> {
    Ok(if input.arbitrary()? {
        Side::Left
    } else {
        Side::Right
    })
}

fn corner(input: &mut Unstructured, sketch: &Sketch) -> Result<Option<Corner>> {
    let corners = sketch.fillet_corners();
    Ok(match input.int_in_range(0u8..=2)? {
        0 if !corners.is_empty() => Some(*input.choose(&corners)?),
        1 => {
            let at = pick_from(input, &points(sketch), sketch)?;
            sketch.corner_at(at).ok()
        }
        _ => {
            let (first, second) = (curve(input, sketch)?, curve(input, sketch)?);
            sketch.corner_between(first, second).ok()
        }
    })
}

fn chamfer_size(input: &mut Unstructured) -> Result<ChamferSize> {
    Ok(match input.int_in_range(0u8..=2)? {
        0 => ChamferSize::Equal(dimensioned_length(input)?),
        1 => ChamferSize::Distances {
            first: dimensioned_length(input)?,
            second: dimensioned_length(input)?,
        },
        _ => ChamferSize::DistanceAndAngle {
            distance: dimensioned_length(input)?,
            angle: dimensioned_angle(input)?,
        },
    })
}

fn picks(input: &mut Unstructured, sketch: &Sketch) -> Result<Option<[Pick; 2]>> {
    let (first, second) = (curve(input, sketch)?, curve(input, sketch)?);

    if input.arbitrary()? {
        return Ok(sketch.crossing_picks(first, second).ok());
    }
    Ok(Some([
        Pick {
            curve: first,
            near: point(input)?,
        },
        Pick {
            curve: second,
            near: point(input)?,
        },
    ]))
}

fn split_work(sketch: &Sketch, curve: EntityId) -> usize {
    sketch
        .entity(curve)
        .map_or(1, |entity| entity.points().len().max(1))
}

fn window<'a>(
    input: &mut Unstructured,
    loose: &'a [EntityId],
    reach: usize,
) -> Result<&'a [EntityId]> {
    let spare = loose.len().saturating_sub(reach);

    if spare == 0 {
        return Ok(loose);
    }

    let start = input.int_in_range(0..=spare)?;
    Ok(loose.get(start..start + reach).unwrap_or_default())
}

fn split_at(input: &mut Unstructured, sketch: &Sketch) -> Result<(EntityId, EntityId)> {
    let drawn = curves(sketch);
    let loose = points(sketch);

    let every_pair = loose
        .len()
        .saturating_mul(drawn.iter().map(|curve| split_work(sketch, *curve)).sum());
    let (candidates, near) = if every_pair <= MOST_SPLIT_WORK {
        (drawn, loose.as_slice())
    } else {
        let chosen = curve(input, sketch)?;
        let reach = MOST_SPLIT_WORK / split_work(sketch, chosen);
        (vec![chosen], window(input, &loose, reach)?)
    };

    let on_curves: Vec<(EntityId, EntityId)> = near
        .iter()
        .flat_map(|point| candidates.iter().map(move |curve| (*curve, *point)))
        .filter(|(curve, point)| sketch.check_split(*curve, *point).is_ok())
        .collect();

    if on_curves.is_empty() || input.ratio(1u8, 4)? {
        return Ok((curve(input, sketch)?, pick_from(input, &loose, sketch)?));
    }
    Ok(*input.choose(&on_curves)?)
}

pub fn edit(input: &mut Unstructured, sketch: &mut Sketch) -> Result<(EditKind, bool)> {
    Ok(match input.int_in_range(0u8..=9)? {
        0 => {
            let (on, near) = (curve(input, sketch)?, point(input)?);

            let placed = sketch
                .closest_on_curve(on, near)
                .map(|position| sketch.add_point(position));
            (EditKind::PointOnCurve, placed.is_some())
        }
        1 => {
            let (cut, near) = (curve(input, sketch)?, point(input)?);
            (EditKind::Trim, sketch.trim(cut, near).is_ok())
        }
        2 => {
            let (extended, near) = (curve(input, sketch)?, point(input)?);
            (EditKind::Extend, sketch.extend(extended, near).is_ok())
        }
        3 => {
            let (split, at) = split_at(input, sketch)?;
            (EditKind::Split, sketch.split_at(split, at).is_ok())
        }
        4 => {
            let broken = if input.arbitrary()? {
                let single = curve(input, sketch)?;
                sketch.break_curve(single).is_ok()
            } else {
                let several = some_curves(input, sketch)?;
                sketch.break_curves(&several).is_ok()
            };
            (EditKind::Break, broken)
        }
        5 => {
            let chosen = if input.arbitrary()? {
                let through = curve(input, sketch)?;
                sketch.offset_chain_through(through)
            } else {
                some_curves(input, sketch)?
            };
            let (towards, by) = (side(input)?, dimensioned_length(input)?);

            let offset = sketch.offset(&chosen, towards, by.value, by.expression);
            (EditKind::Offset, offset.is_ok())
        }
        6 => {
            let chosen = some_curves(input, sketch)?;
            let by = dimensioned_length(input)?;
            let caps = if input.arbitrary()? {
                Caps::Round
            } else {
                Caps::Flat
            };

            let offset = sketch.offset_both_sides(&chosen, by.value, by.expression, caps);
            (EditKind::OffsetBothSides, offset.is_ok())
        }
        7 => {
            let (Some(rounded), radius) = (corner(input, sketch)?, dimensioned_length(input)?)
            else {
                return Ok((EditKind::Fillet, false));
            };

            let filleted = sketch.fillet(&rounded, radius.value, radius.expression);
            (EditKind::Fillet, filleted.is_ok())
        }
        8 => {
            let (Some(cut), size) = (corner(input, sketch)?, chamfer_size(input)?) else {
                return Ok((EditKind::Chamfer, false));
            };
            (EditKind::Chamfer, sketch.chamfer(&cut, &size).is_ok())
        }
        _ => {
            let Some(picked) = picks(input, sketch)? else {
                return Ok((EditKind::Join, false));
            };
            (EditKind::Join, sketch.join_at_crossing(picked).is_ok())
        }
    })
}

pub fn assert_uses_counted(sketch: &Sketch, after: EditKind) {
    let from_scratch = uses_from_scratch(sketch);

    assert_eq!(
        kept_uses(sketch),
        &from_scratch,
        "the use counts kept through a {after:?} differ from a count from scratch"
    );

    for used in from_scratch.keys() {
        assert!(
            sketch.contains(*used),
            "a {after:?} left {used:?} in use by the sketch but removed"
        );
    }
}

pub fn edited_sketch(input: &mut Unstructured, plane: Plane) -> Result<Tally> {
    let mut drawn = sketch::sketch(input, plane)?;
    let mut tally = Tally::default();

    for _ in 0..input.int_in_range(1..=MOST_EDITS)? {
        let before = drawn.clone();
        let (kind, applied) = edit(input, &mut drawn)?;

        if applied {
            *tally.applied.entry(kind).or_default() += 1;
            assert_uses_counted(&drawn, kind);
        } else {
            *tally.refused.entry(kind).or_default() += 1;

            assert_eq!(drawn, before, "a refused {kind:?} changed the sketch");
        }
    }
    Ok(tally)
}
