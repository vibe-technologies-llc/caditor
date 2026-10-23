use caditor_document::{
    CoilLength, CoilTurning, CopyOrientation, CurvePattern, CurveSpacing, DEFAULT_PITCH,
    DEFAULT_TURNS, Evaluation, FeatureId, FeatureKind, FeatureResult, Helix, HelixKind, Pattern,
    PatternKind, PointReference, PointsPattern, is_curve_path,
};
use caditor_expression::Expression;
use caditor_sketch::Sketch;
use serde::Deserialize;

use crate::{
    coil_tools, helix_tools,
    mcp::{
        features::{
            Amount, Built, CopyName, PatternResult, added, count, kept_subject, length, parsed,
            pattern_built, pattern_outcome, sketch_of, skipped_copies, turn,
        },
        places::{self, AxisChoice, PointChoice},
        references::Named,
        session::{Session, Work},
        sweeps::coil_length,
        tools::{Answer, ToolError},
    },
    pattern_tools::{self, Shape},
    sketch_pattern_tools,
    units::LengthUnit,
};

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Helixing {
    pub axis: Option<AxisChoice>,
    pub reversed: Option<bool>,
    pub flat: Option<bool>,
    pub diameter: Option<Amount>,
    pub turns: Option<Amount>,
    pub pitch: Option<Amount>,
    pub height: Option<Amount>,
    pub outer_diameter: Option<Amount>,
    pub taper: Option<Amount>,
    pub start_angle: Option<Amount>,
    pub start_point: Option<PointChoice>,
    pub clockwise: Option<bool>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurvePatterning {
    pub body: Option<Named>,
    pub features: Option<Vec<Named>>,
    pub sketch: Option<Named>,
    pub count: Option<Amount>,
    pub spacing: Option<Amount>,
    pub spread: Option<bool>,
    pub follow: Option<bool>,
    pub reversed: Option<bool>,
    pub skipped: Option<Vec<u32>>,
    pub result: Option<PatternResult>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PointPatterning {
    pub body: Option<Named>,
    pub features: Option<Vec<Named>>,
    pub sketch: Option<Named>,
    pub from: Option<PointChoice>,
    pub skipped: Option<Vec<u32>>,
    pub result: Option<PatternResult>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CurveToolError {
    #[error(
        "the curves of {sketch} do not join end to end into one chain to follow; make the other \
         curves construction geometry"
    )]
    NotAPath { sketch: String },
    #[error("{sketch} has no lone points to place copies at; draw points with add_sketch_geometry")]
    NoPoints { sketch: String },
    #[error("a flat spiral has no height; give its outer_diameter instead")]
    FlatHeight,
    #[error("only a flat spiral has an outer diameter; give the helix's height, or flat: true")]
    HelixOuterDiameter,
}

fn solved(evaluation: &Evaluation, sketch: FeatureId) -> Option<&Sketch> {
    evaluation
        .feature(sketch)
        .and_then(|status| status.result.as_deref())
        .and_then(FeatureResult::sketch)
        .map(|result| &result.geometry)
}

pub fn helix_built(
    session: &mut Session,
    work: &Work<'_>,
    asked: Helixing,
    base: Option<&Helix>,
) -> Result<Built, ToolError> {
    let diameter = parsed(session, asked.diameter.as_ref(), |session, size| {
        length(session, "diameter", size)
    })?;
    let turns = parsed(session, asked.turns.as_ref(), |session, turns| {
        count(session, "turns", turns)
    })?;
    let pitch = parsed(session, asked.pitch.as_ref(), |session, pitch| {
        length(session, "pitch", pitch)
    })?;
    let height = parsed(session, asked.height.as_ref(), |session, height| {
        length(session, "height", height)
    })?;
    let outer = parsed(session, asked.outer_diameter.as_ref(), |session, outer| {
        length(session, "outer_diameter", outer)
    })?;
    let taper = parsed(session, asked.taper.as_ref(), |session, taper| {
        turn(session, "taper", taper)
    })?;
    let start = parsed(session, asked.start_angle.as_ref(), |session, start| {
        turn(session, "start_angle", start)
    })?;
    session.evaluate(work)?;
    let unit = LengthUnit::Millimetre;
    let mut helix = match base {
        Some(base) => base.clone(),
        None => Helix {
            length: CoilLength::TurnsAndPitch {
                turns: Expression::number(DEFAULT_TURNS),
                pitch: unit.default_length(DEFAULT_PITCH),
            },
            ..Helix::new(
                coil_tools::DEFAULT_AXIS,
                unit.default_length(coil_tools::DEFAULT_DIAMETER),
            )
        },
    };
    if let Some(choice) = &asked.axis {
        helix.axis = places::axis(session.document(), session.evaluation(), choice)?;
    }
    if let Some(reversed) = asked.reversed {
        helix.reversed = reversed;
    }
    if let Some(flat) = asked.flat {
        helix.kind = if flat {
            HelixKind::FlatSpiral
        } else {
            HelixKind::Helix
        };
    }
    let reach = match (helix.is_flat(), height, outer) {
        (true, Some(_), _) => return Err(CurveToolError::FlatHeight.into()),
        (false, _, Some(_)) => return Err(CurveToolError::HelixOuterDiameter.into()),
        (true, None, outer) => outer,
        (false, height, None) => height,
    };
    if let Some(diameter) = diameter {
        helix.diameter = diameter;
    }
    if let Some(taper) = taper {
        helix.taper = taper;
    }
    if let Some(start) = start {
        helix.start = start;
        helix.start_point = None;
    }
    if let Some(choice) = &asked.start_point {
        helix.start_point = Some(places::point(
            session.document(),
            session.evaluation(),
            choice,
        )?);
    }
    helix.length = coil_length(&helix.length, (turns, pitch, reach))?;
    if let Some(clockwise) = asked.clockwise {
        helix.turning = if clockwise {
            CoilTurning::Clockwise
        } else {
            CoilTurning::CounterClockwise
        };
    }
    let title = helix_tools::title(helix.kind);
    Ok(Built::new(FeatureKind::Helix(helix), title, asked.name))
}

pub fn helix(session: &mut Session, work: &Work<'_>, asked: Helixing) -> Result<Answer, ToolError> {
    let built = helix_built(session, work, asked, None)?;
    added(session, work, built)
}

fn in_row(skipped: Option<&[u32]>) -> Option<Vec<CopyName>> {
    skipped.map(|copies| copies.iter().copied().map(CopyName::InRow).collect())
}

pub fn curve_pattern_built(
    session: &mut Session,
    work: &Work<'_>,
    asked: CurvePatterning,
    base: Option<&Pattern>,
) -> Result<Built, ToolError> {
    let copies = parsed(session, asked.count.as_ref(), |session, given| {
        count(session, "count", given)
    })?;
    let spacing = parsed(session, asked.spacing.as_ref(), |session, given| {
        length(session, "spacing", given)
    })?;
    session.evaluate(work)?;
    let chosen = kept_subject(
        session,
        (asked.body.as_ref(), asked.features.as_deref()),
        base.map(|base| (base.body, base.repeated.as_slice())),
        "repeat",
    )?;
    let kept = match base.map(|base| &base.kind) {
        Some(PatternKind::Curve(curve)) => Some(curve),
        Some(_) | None => None,
    };
    let document = session.document();
    let (sketch, sketch_name, _) = sketch_of(
        document,
        asked.sketch.as_ref(),
        kept.map(|kept| kept.sketch),
    )?;
    if !solved(session.evaluation(), sketch).is_some_and(is_curve_path) {
        return Err(CurveToolError::NotAPath {
            sketch: sketch_name.to_owned(),
        }
        .into());
    }
    let mut curve = match (
        kept,
        sketch_pattern_tools::kind(Shape::Curve, sketch, LengthUnit::Millimetre),
    ) {
        (Some(kept), _) => CurvePattern {
            sketch,
            ..kept.clone()
        },
        (None, PatternKind::Curve(fresh)) => fresh,
        (None, _) => {
            return Err(CurveToolError::NotAPath {
                sketch: sketch_name.to_owned(),
            }
            .into());
        }
    };
    if let Some(copies) = copies {
        curve.count = copies;
    }
    if let Some(spacing) = spacing {
        curve.spacing = spacing;
    }
    curve.measured = match (asked.spread, asked.spacing.is_some()) {
        (Some(true), _) => CurveSpacing::Spread,
        (Some(false), _) | (None, true) => CurveSpacing::Distance,
        (None, false) => curve.measured,
    };
    if let Some(follow) = asked.follow {
        curve.orientation = if follow {
            CopyOrientation::Following
        } else {
            CopyOrientation::Kept
        };
    }
    if let Some(reversed) = asked.reversed {
        curve.reversed = reversed;
    }
    let skipped = skipped_copies(in_row(asked.skipped.as_deref()).as_deref(), base)?;
    Ok(pattern_built(
        session,
        chosen,
        (
            PatternKind::Curve(curve),
            skipped,
            pattern_outcome(asked.result, base),
        ),
        (asked.name, pattern_tools::CURVE_TITLE),
    ))
}

pub fn curve_pattern(
    session: &mut Session,
    work: &Work<'_>,
    asked: CurvePatterning,
) -> Result<Answer, ToolError> {
    let built = curve_pattern_built(session, work, asked, None)?;
    added(session, work, built)
}

pub fn point_pattern_built(
    session: &mut Session,
    work: &Work<'_>,
    asked: PointPatterning,
    base: Option<&Pattern>,
) -> Result<Built, ToolError> {
    session.evaluate(work)?;
    let chosen = kept_subject(
        session,
        (asked.body.as_ref(), asked.features.as_deref()),
        base.map(|base| (base.body, base.repeated.as_slice())),
        "repeat",
    )?;
    let kept = match base.map(|base| &base.kind) {
        Some(PatternKind::Points(points)) => Some(points),
        Some(_) | None => None,
    };
    let document = session.document();
    let evaluation = session.evaluation();
    let (sketch, sketch_name, _) = sketch_of(
        document,
        asked.sketch.as_ref(),
        kept.map(|kept| kept.sketch),
    )?;
    let has_points =
        solved(evaluation, sketch).is_some_and(|solved| !solved.layout_points().is_empty());
    if !has_points {
        return Err(CurveToolError::NoPoints {
            sketch: sketch_name.to_owned(),
        }
        .into());
    }
    let from = match (&asked.from, kept) {
        (Some(choice), _) => places::point(document, evaluation, choice)?,
        (None, Some(kept)) => kept.base.clone(),
        (None, None) => PointReference::Origin,
    };
    let skipped = skipped_copies(in_row(asked.skipped.as_deref()).as_deref(), base)?;
    Ok(pattern_built(
        session,
        chosen,
        (
            PatternKind::Points(PointsPattern { sketch, base: from }),
            skipped,
            pattern_outcome(asked.result, base),
        ),
        (asked.name, pattern_tools::POINTS_TITLE),
    ))
}

pub fn point_pattern(
    session: &mut Session,
    work: &Work<'_>,
    asked: PointPatterning,
) -> Result<Answer, ToolError> {
    let built = point_pattern_built(session, work, asked, None)?;
    added(session, work, built)
}
