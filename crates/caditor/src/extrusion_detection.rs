use std::time::Instant;

use caditor_document::{
    BodyOperation, Document, Extrude, ExtrudeEnd, ExtrudeExtent, FeatureId, SolidFeature,
};

use crate::{
    body_reach::{self, REACH_BUDGET, Reaches, Verdict},
    solid_tools::Reach,
};

#[derive(Debug, Clone, PartialEq)]
pub struct Detected {
    pub operation: BodyOperation,
    pub extent: ExtrudeExtent,
    pub told: Option<String>,
}

pub fn extrusion(
    document: &Document,
    reach: Reach<'_>,
    template: &Extrude,
    standing_on: Option<FeatureId>,
    chosen: Option<FeatureId>,
) -> Detected {
    let deadline = Instant::now() + REACH_BUDGET;
    let footprint_of = standing_on.filter(|body| chosen.is_none_or(|chosen| chosen == *body));
    let sweep = |reversed: bool| {
        let solid = SolidFeature::Extrude(directed(template, reversed));
        body_reach::reaches(
            document,
            reach.evaluation,
            reach.parameters,
            &solid,
            deadline,
            footprint_of,
        )
    };
    let label = |body: FeatureId| {
        document
            .body_label(body)
            .unwrap_or_else(|| "the body".to_owned())
    };

    let cutting = standing_on.filter(|body| chosen.is_none_or(|chosen| chosen == *body));
    let mut told = None;
    if let Some(body) = cutting {
        let cut = |through: bool| Detected {
            operation: BodyOperation::Remove(body),
            extent: cut_extent(template, through),
            told: None,
        };
        match sweep(true) {
            None => return cut(false),
            Some(reaches) => match reaches.verdict(body) {
                Verdict::Overlaps(reached) if reached.spans_footprint == Some(true) => {
                    told = Some(format!(
                        "The sketch spans the whole face of {}, so the extrusion adds to it instead of cutting.",
                        label(body)
                    ));
                }
                Verdict::Overlaps(reached) => return cut(reaches.clears(&reached)),
                Verdict::Unknown => return cut(false),
                Verdict::Misses => {
                    told = Some(format!(
                        "The sketch lies off {}, so the extrusion does not cut into it.",
                        label(body)
                    ));
                }
            },
        }
    }

    let joining = |found: &Option<Reaches>| {
        let found = found.as_ref()?;
        let reached = found.reaching(chosen)?;
        Some(
            match found.meets_only_along_an_edge(reach.evaluation, reached) {
                true => Err(reached.body),
                false => Ok(reached.body),
            },
        )
    };
    let forward = sweep(false);
    let edgewise = match joining(&forward) {
        Some(Ok(body)) => {
            return Detected {
                operation: BodyOperation::Add(body),
                extent: directed(template, false).extent,
                told,
            };
        }
        Some(Err(body)) => Some(body),
        None => None,
    };
    if forward.is_some()
        && let Some(Ok(body)) = joining(&sweep(true))
    {
        let other_way = if edgewise == Some(body) {
            "would meet it only along an edge"
        } else {
            "reached no body"
        };
        return Detected {
            operation: BodyOperation::Add(body),
            extent: directed(template, true).extent,
            told: Some(format!(
                "The extrusion points toward {}; the other way {other_way}.",
                label(body)
            )),
        };
    }
    let operation = chosen.map_or(BodyOperation::NewBody, BodyOperation::Add);
    if let (BodyOperation::NewBody, Some(body)) = (operation, edgewise) {
        told = Some(format!(
            "The extrusion would meet {} only along an edge, so it makes a new body.",
            label(body)
        ));
    }
    Detected {
        operation,
        extent: directed(template, false).extent,
        told,
    }
}

fn directed(template: &Extrude, reversed: bool) -> Extrude {
    let mut directed = template.clone();
    if let ExtrudeExtent::OneSide {
        reversed: current, ..
    } = &mut directed.extent
    {
        *current = reversed;
    }
    directed
}

fn cut_extent(template: &Extrude, through: bool) -> ExtrudeExtent {
    let mut extent = directed(template, true).extent;
    if let (true, ExtrudeExtent::OneSide { end, .. }) = (through, &mut extent) {
        *end = ExtrudeEnd::ThroughAll;
    }
    extent
}
