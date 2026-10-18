use caditor_expression::{Expression, ParameterId};

use crate::{
    body_appearance::BodyAppearance,
    configurations::{Configurations, ConfiguredValue, Setting},
    document::{Document, FeatureKind, Parameter},
    edit::{Edit, EditError, Transaction},
    pattern::PatternKind,
    solid::{ExtrudeExtent, RevolveExtent, SolidFeature, SolidStart},
};

impl Document {
    pub fn inline_parameter(&self, id: ParameterId) -> Result<Transaction, EditError> {
        let parameter = self.parameter(id).ok_or(EditError::MissingParameter)?;
        if let Some(measurement) = self.measurement_of(id) {
            return Err(EditError::MeasuredParameterInlined {
                name: parameter.name.clone(),
                measurement: measurement.name.clone(),
            });
        }
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
                for (text, held) in sketch.sketch.texts() {
                    if held.uses(id) {
                        let mut inlined = held.clone();
                        for expression in [&mut inlined.height, &mut inlined.angle] {
                            if expression.uses(id) {
                                *expression = inline(expression, &feature.name)?;
                            }
                        }
                        edits.push(Edit::SetSketchText {
                            feature: feature.id(),
                            id: text,
                            text: inlined,
                        });
                    }
                }
                for (gear, held) in sketch.sketch.gears() {
                    if held.uses(id) {
                        let mut inlined = held.clone();
                        for expression in inlined.expressions_mut() {
                            if expression.uses(id) {
                                *expression = inline(expression, &feature.name)?;
                            }
                        }
                        edits.push(Edit::SetSketchGear {
                            feature: feature.id(),
                            id: gear,
                            gear: inlined,
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
            for (key, look) in feature.piece_looks.appearances() {
                let (Some(density), Some(piece)) = (&look.density, feature.id().piece(key)) else {
                    continue;
                };
                if density.uses(id) {
                    let appearance = BodyAppearance {
                        density: Some(inline(density, &feature.name)?),
                        ..look.clone()
                    };
                    edits.push(Edit::SetBodyAppearance {
                        id: piece,
                        appearance,
                    });
                }
            }
        }
        edits.extend(self.inlined_in_configurations(parameter)?);
        edits.push(Edit::RemoveParameter { id });
        Ok(Transaction::new(
            format!("Delete {}", parameter.name),
            edits,
        ))
    }

    fn inlined_in_configurations(&self, parameter: &Parameter) -> Result<Option<Edit>, EditError> {
        let id = parameter.id();
        let mut table = Configurations::clone(self.configurations());
        let column = ConfiguredValue::Parameter(id);
        let named = table.configures(column) || !self.configuration_users(id).is_empty();
        if !named {
            return Ok(None);
        }
        let active = table.active;
        for row in &mut table.rows {
            if active == Some(row.id) {
                continue;
            }
            let own = match row.settings.get(&column) {
                Some(Setting::Expression(expression)) => expression.clone(),
                _ => parameter.expression.clone(),
            };
            for setting in row.settings.values_mut() {
                if let Setting::Expression(expression) = setting
                    && expression.uses(id)
                {
                    *expression =
                        expression
                            .inlining(id, &own)
                            .map_err(|_| EditError::InliningTooLong {
                                name: parameter.name.clone(),
                                user: format!("{} configuration", row.name),
                            })?;
                }
            }
        }
        table.values.retain(|value| *value != column);
        for row in &mut table.rows {
            row.settings.remove(&column);
        }
        Ok(Some(Edit::SetConfigurations {
            configurations: Box::new(table),
        }))
    }
}

pub(crate) fn expressions_mut(kind: &mut FeatureKind) -> Vec<&mut Expression> {
    match kind {
        FeatureKind::Solid(SolidFeature::Extrude(extrude)) => {
            let mut expressions = match &mut extrude.extent {
                ExtrudeExtent::Symmetric { distance } => vec![distance],
                ExtrudeExtent::OneSide { end, .. } => end.expressions_mut(),
                ExtrudeExtent::TwoSides { forward, backward } => {
                    let mut ends = forward.expressions_mut();
                    ends.extend(backward.expressions_mut());
                    ends
                }
            };
            expressions.extend(start_distance(&mut extrude.start));
            expressions.extend(extrude.taper.as_deref_mut());
            expressions.extend(extrude.wall.as_deref_mut().map(|wall| &mut wall.thickness));
            expressions
        }
        FeatureKind::Solid(SolidFeature::Revolve(revolve)) => {
            let mut expressions = match &mut revolve.extent {
                RevolveExtent::Full | RevolveExtent::UpTo { .. } => Vec::new(),
                RevolveExtent::OneSide { angle, .. } | RevolveExtent::Symmetric { angle } => {
                    vec![angle]
                }
                RevolveExtent::TwoSides { forward, backward } => vec![forward, backward],
            };
            expressions.extend(start_distance(&mut revolve.start));
            expressions.extend(revolve.wall.as_deref_mut().map(|wall| &mut wall.thickness));
            expressions
        }
        FeatureKind::Solid(SolidFeature::Sweep(sweep)) => sweep.expressions_mut(),
        FeatureKind::Blend(blend) => blend.expressions_mut(),
        FeatureKind::Shell(shell) => shell.expressions_mut(),
        FeatureKind::OffsetFace(offset) => vec![&mut offset.distance],
        FeatureKind::MoveFace(moving) => moving.expressions_mut().into(),
        FeatureKind::DeleteFace(_) => Vec::new(),
        FeatureKind::Emboss(emboss) => vec![&mut emboss.depth],
        FeatureKind::Primitive(primitive) => primitive.expressions_mut(),
        FeatureKind::Pipe(pipe) => pipe.expressions_mut(),
        FeatureKind::Loft(loft) => loft.expressions_mut(),
        FeatureKind::Coil(coil) => coil.expressions_mut(),
        FeatureKind::Helix(helix) => helix.expressions_mut(),
        FeatureKind::Rib(rib) => rib.expressions_mut(),
        FeatureKind::Thread(thread) => thread.expressions_mut(),
        FeatureKind::Move(movement) => movement.expressions_mut().collect(),
        FeatureKind::Mate(mate) => mate.expressions_mut(),
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
            PatternKind::Curve(curve) => vec![&mut curve.count, &mut curve.spacing],
            PatternKind::Points(_) => Vec::new(),
        },
        FeatureKind::Datum(datum) => datum.expressions_mut(),
        FeatureKind::Import(import) => import.placement.expressions_mut().collect(),
        FeatureKind::ReferenceImage(image) => image.expressions_mut(),
        FeatureKind::Sketch(_)
        | FeatureKind::Combine(_)
        | FeatureKind::Mirror(_)
        | FeatureKind::Split(_)
        | FeatureKind::SplitFace(_)
        | FeatureKind::Remove(_)
        | FeatureKind::Measurement(_) => Vec::new(),
    }
}

fn start_distance(start: &mut Option<SolidStart>) -> Option<&mut Expression> {
    match start {
        Some(SolidStart::Distance(distance)) => Some(distance),
        Some(SolidStart::Plane(_)) | None => None,
    }
}
