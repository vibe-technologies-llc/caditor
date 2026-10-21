use std::time::Instant;

use caditor_document::{
    BodyOperation, Document, Extrude, ExtrudeEnd, ExtrudeExtent, FeatureId, SolidFeature,
};

use crate::{
    body_reach::{self, Meeting, REACH_BUDGET, Reaches, Verdict},
    solid_tools::Reach,
};

const CHANGE_RESULT: &str = "Result in the panel changes that.";
const CHANGE_END: &str = "End in the panel sets a distance instead.";

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
        let cut = |through: bool, told: Option<String>| Detected {
            operation: BodyOperation::Remove(body),
            extent: cut_extent(template, through),
            told,
        };
        let unsure = || {
            Some(format!(
                "Could not tell where the sketch reaches {}, so the extrusion cuts into it. {CHANGE_RESULT}",
                label(body)
            ))
        };
        match sweep(true) {
            None => return cut(false, unsure()),
            Some(reaches) => match reaches.verdict(body) {
                Verdict::Overlaps(reached) if reached.spans_footprint == Some(true) => {
                    told = Some(format!(
                        "The sketch spans the whole face of {}, so the extrusion adds to it instead of cutting.",
                        label(body)
                    ));
                }
                Verdict::Overlaps(reached) => {
                    let through = reaches.clears(&reached);
                    let told = through.then(|| {
                        format!(
                            "The cut passes right through {}, so it ends at Through all. {CHANGE_END}",
                            label(body)
                        )
                    });
                    return cut(through, told);
                }
                Verdict::Unknown => return cut(false, unsure()),
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
        Some((reached.body, found.meeting(reach.evaluation, reached)))
    };
    let forward = sweep(false);
    let edgewise = match joining(&forward) {
        Some((body, meeting @ (Meeting::Joins | Meeting::Unchecked))) => {
            let note = match meeting {
                Meeting::Unchecked => Some(format!(
                    "The extrusion meets {}, so it is added to it. There was no time to check whether it meets it only along an edge, which cannot be joined. {CHANGE_RESULT}",
                    label(body)
                )),
                _ if told.is_none() && chosen.is_none() => Some(format!(
                    "The extrusion meets {}, so it is added to it. {CHANGE_RESULT}",
                    label(body)
                )),
                _ => None,
            };
            return Detected {
                operation: BodyOperation::Add(body),
                extent: directed(template, false).extent,
                told: [told, note]
                    .into_iter()
                    .flatten()
                    .reduce(|first, second| format!("{first} {second}")),
            };
        }
        Some((body, Meeting::OnlyAlongAnEdge)) => Some(body),
        None => None,
    };
    if forward.is_some()
        && let Some((body, Meeting::Joins | Meeting::Unchecked)) = joining(&sweep(true))
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

#[cfg(test)]
mod tests {
    use caditor_document::{Evaluation, FeatureKind};

    use super::*;
    use crate::samples::Sample;

    #[test]
    fn a_cut_whose_reach_could_not_be_checked_says_so_and_where_to_change_it() {
        let document = Sample::Plate.document().unwrap();
        let (plate, template) = document
            .features()
            .find_map(|feature| match &feature.kind {
                FeatureKind::Solid(SolidFeature::Extrude(extrude)) => {
                    Some((feature.id(), extrude.clone()))
                }
                _ => None,
            })
            .unwrap();
        let unevaluated = Evaluation::default();

        let unknown = extrusion(
            &document,
            Reach {
                evaluation: &unevaluated,
                parameters: &unevaluated.parameters,
            },
            &template,
            Some(plate),
            None,
        );

        assert_eq!(unknown.operation, BodyOperation::Remove(plate));
        let told = unknown.told.unwrap();
        assert!(
            told.starts_with("Could not tell where the sketch reaches"),
            "{told}"
        );
        assert!(told.ends_with(CHANGE_RESULT), "{told}");
    }
}
