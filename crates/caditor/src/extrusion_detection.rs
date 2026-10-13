use std::time::Instant;

use caditor_document::{
    BodyOperation, Document, Extrude, ExtrudeEnd, ExtrudeExtent, FeatureId, SolidFeature,
};

use crate::{
    body_reach::{self, REACH_BUDGET, Verdict},
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
    let sweep = |reversed: bool| {
        let solid = SolidFeature::Extrude(directed(template, reversed));
        body_reach::reaches(
            document,
            reach.evaluation,
            reach.parameters,
            &solid,
            deadline,
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

    let forward = sweep(false);
    if let Some(reached) = forward.as_ref().and_then(|found| found.reaching(chosen)) {
        return Detected {
            operation: BodyOperation::Add(reached.body),
            extent: directed(template, false).extent,
            told,
        };
    }
    if forward.is_some() {
        let backward = sweep(true);
        if let Some(reached) = backward.as_ref().and_then(|found| found.reaching(chosen)) {
            return Detected {
                operation: BodyOperation::Add(reached.body),
                extent: directed(template, true).extent,
                told: Some(format!(
                    "The extrusion points toward {}; the other way reached no body.",
                    label(reached.body)
                )),
            };
        }
    }
    Detected {
        operation: chosen.map_or(BodyOperation::NewBody, BodyOperation::Add),
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
