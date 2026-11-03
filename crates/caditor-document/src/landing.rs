use std::{collections::BTreeMap, sync::Arc};

use caditor_geometry::{Plane, Similarity, Vector3};
use caditor_kernel::{BooleanOperation, PatternCopy, PatternError, Solid, pattern_copies};

use crate::{
    document::{Feature, FeatureId, FeatureKind},
    hole::{self, Hole},
    pattern::Instance,
    recompute::{CancelToken, Failure, FeatureError, FeatureResult, FixTarget, Inputs},
    repeat_blend::{Repeat, RepeatCause},
    solid::{self, Extrude, SolidFeature},
};

pub(crate) struct Landing<'a> {
    pub repeater: FeatureId,
    pub copy: Instance,
    pub placement: Similarity,
    pub body: FeatureId,
    pub standing: &'a Solid,
    pub operation: BooleanOperation,
}

#[derive(Debug)]
pub(crate) struct LandingInputs {
    features: BTreeMap<FeatureId, Arc<FeatureResult>>,
    bodies: BTreeMap<FeatureId, (FeatureId, Arc<FeatureResult>)>,
}

impl LandingInputs {
    pub(crate) fn of(inputs: &Inputs<'_>) -> Self {
        Self {
            features: inputs.features.clone(),
            bodies: inputs.bodies.clone(),
        }
    }

    pub(crate) fn results(&self) -> impl Iterator<Item = &Arc<FeatureResult>> {
        self.features
            .values()
            .chain(self.bodies.values().map(|(_, result)| result))
    }

    fn inputs<'a>(&'a self, outer: &Inputs<'a>) -> Inputs<'a> {
        Inputs {
            document: outer.document,
            parameters: outer.parameters,
            features: &self.features,
            bodies: &self.bodies,
            previous: None,
        }
    }
}

impl PartialEq for LandingInputs {
    fn eq(&self, other: &Self) -> bool {
        let same_features = self.features.len() == other.features.len()
            && self.features.iter().zip(&other.features).all(
                |((id, result), (other_id, other))| id == other_id && Arc::ptr_eq(result, other),
            );
        let same_bodies = self.bodies.len() == other.bodies.len()
            && self.bodies.iter().zip(&other.bodies).all(
                |((id, (state, result)), (other_id, (other_state, other)))| {
                    id == other_id && state == other_state && Arc::ptr_eq(result, other)
                },
            );
        same_features && same_bodies
    }
}

pub(crate) struct Landed {
    pub body: Solid,
    pub tools: Vec<Solid>,
}

impl Landing<'_> {
    pub(crate) fn mirrored(&self) -> bool {
        self.placement.is_mirrored()
    }

    pub(crate) fn plane(&self, plane: &Plane) -> Plane {
        let mapped = plane.mapped(&self.placement);
        if self.mirrored() {
            mapped.flipped()
        } else {
            mapped
        }
    }

    pub(crate) fn direction(&self, direction: Vector3) -> Vector3 {
        let mapped = self.placement.apply_direction(direction);
        if self.mirrored() { -mapped } else { mapped }
    }

    pub(crate) fn body(&self, body: FeatureId) -> Option<&Solid> {
        (body == self.body).then_some(self.standing)
    }

    pub(crate) fn named(&self, tool: &Solid) -> Result<Solid, Failure> {
        let in_place = PatternCopy {
            index: self.copy,
            placement: Similarity::IDENTITY,
        };
        match pattern_copies(tool, &[in_place], self.repeater.raw()) {
            Ok(Some(named)) => Ok(named),
            Err(PatternError::Cancelled(_)) => Err(Failure::Cancelled),
            Ok(None) | Err(PatternError::Placement { .. } | PatternError::Union { .. }) => {
                Err(Failure::from(FeatureError::new(RepeatCause::CopyUnnamed)))
            }
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Lander<'a> {
    Extrude {
        solid: &'a SolidFeature,
        extrude: &'a Extrude,
    },
    Hole(&'a Hole),
}

impl<'a> Lander<'a> {
    pub(crate) fn of(kind: &'a FeatureKind) -> Option<Self> {
        if let Some(hole) = kind.hole() {
            let lands = !hole.depth.is_blind() && !hole.is_modelled_thread();
            return lands.then_some(Self::Hole(hole));
        }
        let solid = kind.solid()?;
        let SolidFeature::Extrude(extrude) = solid else {
            return None;
        };
        let lands = extrude
            .extent
            .ends()
            .iter()
            .any(|end| end.distance().is_none());
        lands.then_some(Self::Extrude { solid, extrude })
    }
}

pub(crate) struct Copies<'a> {
    pub feature: &'a Feature,
    pub lander: Lander<'a>,
    pub seen: &'a LandingInputs,
    pub name: &'a str,
    pub body: FeatureId,
    pub operation: BooleanOperation,
}

impl Copies<'_> {
    pub(crate) fn land(
        &self,
        inputs: &Inputs<'_>,
        standing: Solid,
        copies: &[PatternCopy],
        repeat: &Repeat<'_>,
        cancel: &CancelToken,
    ) -> Result<Landed, Failure> {
        let seen = self.seen.inputs(inputs);
        let inputs = &seen;
        let mut body = standing;
        let mut tools = Vec::new();
        for copy in copies {
            if cancel.is_cancelled() {
                return Err(Failure::Cancelled);
            }
            let landing = Landing {
                repeater: repeat.feature,
                copy: copy.index,
                placement: copy.placement,
                body: self.body,
                standing: &body,
                operation: self.operation,
            };
            let landed = match self.lander {
                Lander::Extrude { solid, extrude } => {
                    solid::land(self.feature, solid, extrude, inputs, &landing)
                }
                Lander::Hole(definition) => {
                    hole::land(self.feature, definition, inputs, &landing, cancel)
                }
            }
            .map_err(|failure| self.failure(failure, repeat, copy.index))?;
            body = landed.body;
            tools.extend(landed.tools);
        }
        Ok(Landed { body, tools })
    }

    fn failure(&self, failure: Failure, repeat: &Repeat<'_>, copy: Instance) -> Failure {
        let Failure::Error(error) = failure else {
            return failure;
        };
        let place = error.place;
        let cause = RepeatCause::CopyEnd {
            repeater: repeat.repeater,
            name: self.name.to_owned(),
            copy: (repeat.copy)(copy),
            cause: error.cause,
        };
        Failure::from(FeatureError::new(cause).fixing(FixTarget::Feature(repeat.feature)))
            .placed(place)
    }
}
