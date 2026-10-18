use caditor_kernel::ProfileCurve;
use caditor_sketch::{BORE_TOOTH, GearOutline, GearPiece, Sketch, gear_curve};

fn piece_curve(name: u64, piece: &GearPiece) -> Option<ProfileCurve> {
    Some(match piece {
        GearPiece::Line { from, to } => ProfileCurve::line(name, *from, *to),
        GearPiece::Arc {
            center,
            from,
            to,
            counter_clockwise,
            ..
        } => {
            if *counter_clockwise {
                ProfileCurve::arc(name, *center, *from, *to)
            } else {
                ProfileCurve::arc(name, *center, *to, *from)
            }
        }
        GearPiece::Flank { .. } => {
            let spline = piece.spline()?;
            ProfileCurve::spline(
                name,
                spline.degree(),
                spline.knots().to_vec(),
                spline.control_points().to_vec(),
            )
        }
    })
}

fn outline_curves(gear: caditor_sketch::EntityId, outline: &GearOutline) -> Vec<ProfileCurve> {
    let teeth = outline
        .named_pieces()
        .into_iter()
        .filter_map(|(name, piece)| {
            piece_curve(gear_curve(gear, name.tooth, name.segment)?, piece)
        });
    let bore = outline
        .bore
        .and_then(|radius| {
            Some(ProfileCurve::circle(
                gear_curve(gear, BORE_TOOTH, 0)?,
                outline.centre,
                radius,
            ))
        })
        .into_iter();
    teeth.chain(bore).collect()
}

pub(crate) fn gear_curves(sketch: &Sketch) -> impl Iterator<Item = ProfileCurve> + '_ {
    sketch.gears().flat_map(|(id, _)| {
        sketch
            .gear_outline(id)
            .map(|outline| outline_curves(id, &outline))
            .unwrap_or_default()
    })
}
