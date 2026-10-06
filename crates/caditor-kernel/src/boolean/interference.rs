use caditor_geometry::{Aabb, Point3};

use super::{BooleanError, BooleanOperation, Input, Operand, TOLERANCE, boolean};
use crate::{
    interrupt::{self, Interrupted},
    intersect::boxes_overlap,
    measure::{Element, distance},
    topology::{FaceId, Solid},
};

const TOUCHING: f64 = 10.0 * TOLERANCE;

#[derive(Debug, Clone, PartialEq)]
pub enum Interference {
    Apart,
    Touching(Point3),
    Overlapping(Solid),
}

pub fn interference(first: &Solid, second: &Solid) -> Result<Interference, BooleanError> {
    interrupt::check()?;
    let (Some(first_box), Some(second_box)) = (first.bounding_box(), second.bounding_box()) else {
        return Ok(Interference::Apart);
    };
    if !boxes_overlap(&first_box, &second_box, TOUCHING) {
        return Ok(Interference::Apart);
    }
    match boolean(first, second, BooleanOperation::Intersection) {
        Ok(overlap) => Ok(Interference::Overlapping(overlap)),
        Err(BooleanError::Empty) => {
            Ok(touching(first, second)?.map_or(Interference::Apart, Interference::Touching))
        }
        Err(error) => Err(error),
    }
}

#[derive(Debug, Clone, Copy)]
struct Contact {
    place: Point3,
    reach: f64,
    off_centre: f64,
}

impl Contact {
    fn better_than(&self, other: &Self) -> bool {
        if (self.reach - other.reach).abs() > TOUCHING {
            self.reach > other.reach
        } else {
            self.off_centre < other.off_centre
        }
    }
}

fn touching(first: &Solid, second: &Solid) -> Result<Option<Point3>, Interrupted> {
    let input = Input::new(first, second);
    let mut best: Option<Contact> = None;
    for face in input.faces(Operand::First) {
        let reach = face.bounds.expanded(TOUCHING);
        for other in input.faces_near(Operand::Second, &reach) {
            interrupt::check()?;
            let apart = distance(
                Element::Face {
                    solid: first,
                    face: face.id,
                },
                Element::Face {
                    solid: second,
                    face: other.id,
                },
            );
            let Ok(apart) = apart else {
                continue;
            };
            if apart.distance > TOUCHING {
                continue;
            }
            let contact = contact_between(
                (first, face.id, &face.bounds),
                (second, other.id, &other.bounds),
            )
            .unwrap_or(Contact {
                place: apart.from.midpoint(apart.to),
                reach: 0.0,
                off_centre: f64::INFINITY,
            });
            if best.is_none_or(|best| contact.better_than(&best)) {
                best = Some(contact);
            }
        }
    }
    Ok(best.map(|contact| contact.place))
}

fn contact_between(
    (first, first_face, first_bounds): (&Solid, FaceId, &Aabb),
    (second, second_face, second_bounds): (&Solid, FaceId, &Aabb),
) -> Option<Contact> {
    let shared = Aabb::from_points([
        first_bounds.min().max(second_bounds.min()),
        first_bounds.max().min(second_bounds.max()),
    ])?;
    let on_first = distance(
        Element::Point(shared.center()),
        Element::Face {
            solid: first,
            face: first_face,
        },
    )
    .ok()?
    .to;
    let to_second = distance(
        Element::Point(on_first),
        Element::Face {
            solid: second,
            face: second_face,
        },
    )
    .ok()?;
    let place = on_first.midpoint(to_second.to);
    (to_second.distance <= TOUCHING).then(|| Contact {
        place,
        reach: shared.diagonal(),
        off_centre: place.distance(shared.center()),
    })
}
