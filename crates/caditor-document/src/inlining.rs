use caditor_expression::{Expression, ParameterId};

use crate::{
    document::{Document, FeatureKind},
    edit::{Edit, EditError, Transaction},
    pattern::PatternKind,
    solid::{ExtrudeEnd, ExtrudeExtent, RevolveExtent, SolidFeature, SolidStart},
};

impl Document {
    pub fn inline_parameter(&self, id: ParameterId) -> Result<Transaction, EditError> {
        let parameter = self.parameter(id).ok_or(EditError::MissingParameter)?;
        let inline = |expression: &Expression, user: &str| {
            expression
                .inlining(id, &parameter.expression)
                .map_err(|_| EditError::InliningTooLong {
                    name: parameter.name.clone(),
                    user: user.to_owned(),
                })
        };

        let mut edits = Vec::new();
        for other in self.parameters() {
            if other.expression.uses(id) {
                edits.push(Edit::SetParameterExpression {
                    id: other.id(),
                    expression: inline(&other.expression, &other.name)?,
                });
            }
        }
        for feature in self.features() {
            if let FeatureKind::Sketch(sketch) = &feature.kind {
                for (constraint, held) in sketch.sketch.constraints() {
                    if let Some(value) = held.dimension().filter(|value| value.uses(id)) {
                        edits.push(Edit::SetDimension {
                            feature: feature.id(),
                            constraint,
                            value: inline(value, &feature.name)?,
                        });
                    }
                }
            } else if feature.kind.uses_parameter(id) {
                let mut kind = feature.kind.clone();
                for expression in expressions_mut(&mut kind) {
                    if expression.uses(id) {
                        *expression = inline(expression, &feature.name)?;
                    }
                }
                edits.push(Edit::SetFeatureKind {
                    id: feature.id(),
                    kind,
                });
            }
            if feature.appearance.uses_parameter(id) {
                let mut appearance = feature.appearance.clone();
                if let Some(density) = &mut appearance.density {
                    *density = inline(density, &feature.name)?;
                }
                edits.push(Edit::SetBodyAppearance {
                    id: feature.id(),
                    appearance,
                });
            }
        }
        edits.push(Edit::RemoveParameter { id });
        Ok(Transaction::new(
            format!("Delete {}", parameter.name),
            edits,
        ))
    }
}

fn expressions_mut(kind: &mut FeatureKind) -> Vec<&mut Expression> {
    match kind {
        FeatureKind::Solid(SolidFeature::Extrude(extrude)) => {
            let mut expressions = match &mut extrude.extent {
                ExtrudeExtent::Symmetric { distance } => vec![distance],
                ExtrudeExtent::OneSide { end, .. } => end_distance(end).into_iter().collect(),
                ExtrudeExtent::TwoSides { forward, backward } => end_distance(forward)
                    .into_iter()
                    .chain(end_distance(backward))
                    .collect(),
            };
            expressions.extend(start_distance(&mut extrude.start));
            expressions
        }
        FeatureKind::Solid(SolidFeature::Revolve(revolve)) => {
            let mut expressions = match &mut revolve.extent {
                RevolveExtent::Full => Vec::new(),
                RevolveExtent::OneSide { angle, .. } | RevolveExtent::Symmetric { angle } => {
                    vec![angle]
                }
                RevolveExtent::TwoSides { forward, backward } => vec![forward, backward],
            };
            expressions.extend(start_distance(&mut revolve.start));
            expressions
        }
        FeatureKind::Blend(blend) => vec![&mut blend.size],
        FeatureKind::Shell(shell) => vec![&mut shell.thickness],
        FeatureKind::OffsetFace(offset) => vec![&mut offset.distance],
        FeatureKind::Primitive(primitive) => primitive.expressions_mut(),
        FeatureKind::Thread(thread) => thread.expressions_mut(),
        FeatureKind::Move(movement) => movement.expressions_mut().collect(),
        FeatureKind::Scale(scale) => std::iter::once(&mut scale.factor)
            .chain(scale.center.iter_mut())
            .collect(),
        FeatureKind::Hole(hole) => hole.expressions_mut(),
        FeatureKind::Pattern(pattern) => match &mut pattern.kind {
            PatternKind::Linear { first, second } => std::iter::once(first)
                .chain(second.as_mut())
                .flat_map(|direction| [&mut direction.count, &mut direction.spacing])
                .collect(),
            PatternKind::Circular(circular) => vec![&mut circular.count, &mut circular.angle],
        },
        FeatureKind::Datum(datum) => datum.expressions_mut(),
        FeatureKind::Import(import) => import.placement.expressions_mut().collect(),
        FeatureKind::Sketch(_)
        | FeatureKind::Combine(_)
        | FeatureKind::Mirror(_)
        | FeatureKind::Split(_)
        | FeatureKind::Remove(_) => Vec::new(),
    }
}

fn end_distance(end: &mut ExtrudeEnd) -> Option<&mut Expression> {
    match end {
        ExtrudeEnd::Distance(distance) => Some(distance),
        ExtrudeEnd::ThroughAll | ExtrudeEnd::UpToNext | ExtrudeEnd::UpToFace(_) => None,
    }
}

fn start_distance(start: &mut Option<SolidStart>) -> Option<&mut Expression> {
    match start {
        Some(SolidStart::Distance(distance)) => Some(distance),
        Some(SolidStart::Plane(_)) | None => None,
    }
}
