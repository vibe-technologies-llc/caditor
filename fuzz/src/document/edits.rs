use std::collections::BTreeMap;

use caditor_document::{
    BodyAppearance, Configuration, ConfigurationId, Configurations, ConfiguredValue, Edit,
    FaceColour, FeatureId, Finish, ModelProperties, ModelProperty, NamedView, Outline,
    ParameterOwner, PrincipalGeometry, PrincipalPlane, ProjectionSource, Rgb, RollbackBar,
    SavedView, SavedViews, SelectionSet, SelectionSets, SetMember, Setting, SketchAttachment,
    Transaction,
};
use caditor_geometry::{Point3, Rotation3, Vector2};
use caditor_sketch::{ConstraintId, Entity, EntityId, Sketch};
use libfuzzer_sys::arbitrary::{Result, Unstructured};

use super::{
    LABEL,
    kinds::{Family, feature_kind},
    references::Scene,
};
use crate::{
    coordinate, length, plane, point,
    sketch::{constraint, length_value},
};

const NAMES: [&str; 6] = ["width", "depth", "angle", "a", "b", "Sketch"];
const TEXTS: [&str; 9] = [
    "Front",
    "front",
    "  Top\nview  ",
    "",
    "   ",
    "Gruppe Ü ✓",
    "line one\r\nline two",
    "Set 1",
    "Configuration 2",
];
const LONG_TEXT_CHARS: u16 = 2100;
const DISTANCES: [f64; 6] = [50.0, 1.0, 1e-9, 0.0, -1.0, f64::NAN];
const MATERIALS: [&str; 4] = ["Steel", "Aluminium", "", "PLA"];
const DENSITIES: [&str; 3] = ["7.85 g/cm^3", "2700 kg/m^3", "width"];
const MOST_MEMBERS: u8 = 3;
const MOST_ROWS: u8 = 3;

fn single(edit: Edit) -> Option<Transaction> {
    Some(Transaction::single(LABEL, edit))
}

pub fn text(input: &mut Unstructured) -> Result<String> {
    if input.ratio(1u8, 16)? {
        let chars = input.int_in_range(1..=LONG_TEXT_CHARS)?;
        return Ok("é".repeat(usize::from(chars)));
    }
    Ok((*input.choose(&TEXTS)?).to_owned())
}

fn optional_text(input: &mut Unstructured) -> Result<Option<String>> {
    Ok(if input.arbitrary::<bool>()? {
        Some(text(input)?)
    } else {
        None
    })
}

fn chosen<T: Clone>(input: &mut Unstructured, items: &[T]) -> Result<Option<T>> {
    if items.is_empty() {
        return Ok(None);
    }
    Ok(Some(input.choose(items)?.clone()))
}

pub fn addition(input: &mut Unstructured, scene: &Scene) -> Result<Option<Transaction>> {
    let document = scene.document;
    let mut transaction = document.transaction(LABEL);
    if input.ratio(1u8, 5)? {
        let name = *input.choose(&NAMES)?;
        let Some(expression) = scene.expression(input)? else {
            return Ok(None);
        };
        if input.ratio(1u8, 3)? {
            let owner = owner(input, scene)?;
            transaction.add_owned_parameter(name, expression, owner);
        } else {
            transaction.add_parameter(name, expression);
        }
        return Ok(Some(transaction.finish()));
    }
    let family = *input.choose(&Family::ALL)?;
    let Some(kind) = feature_kind(input, scene, family)? else {
        return Ok(None);
    };
    transaction.add_feature(*input.choose(&NAMES)?, kind);
    Ok(Some(transaction.finish()))
}

fn owner(input: &mut Unstructured, scene: &Scene) -> Result<ParameterOwner> {
    Ok(if input.arbitrary::<bool>()? {
        ParameterOwner::Feature {
            feature: scene.feature(input)?,
            value: text(input)?,
        }
    } else {
        let sketch = scene.sketch(input)?;
        ParameterOwner::Dimension {
            sketch,
            constraint: some_constraint(input, scene, sketch)?,
        }
    })
}

pub fn parameter_edit(input: &mut Unstructured, scene: &Scene) -> Result<Option<Transaction>> {
    let Some(id) = scene.parameter(input)? else {
        return Ok(None);
    };
    let count = scene.document.parameters().len();
    Ok(match input.int_in_range(0u8..=5)? {
        0 => single(Edit::RemoveParameter { id }),
        1 => single(Edit::RenameParameter {
            id,
            name: (*input.choose(&NAMES)?).to_owned(),
        }),
        2 => scene
            .expression(input)?
            .and_then(|expression| single(Edit::SetParameterExpression { id, expression })),
        3 => single(Edit::MoveParameter {
            id,
            index: input.int_in_range(0..=count)?,
        }),
        4 => single(Edit::SetParameterNote {
            id,
            note: text(input)?,
        }),
        _ => single(Edit::SetParameterOwner {
            id,
            owner: if input.arbitrary::<bool>()? {
                Some(owner(input, scene)?)
            } else {
                None
            },
        }),
    })
}

fn appearance(input: &mut Unstructured, scene: &Scene, body: FeatureId) -> Result<BodyAppearance> {
    let colour = |input: &mut Unstructured| -> Result<Rgb> {
        Ok(Rgb {
            red: input.arbitrary()?,
            green: input.arbitrary()?,
            blue: input.arbitrary()?,
        })
    };
    let opacity = |input: &mut Unstructured| -> Result<Option<u8>> {
        Ok(if input.arbitrary::<bool>()? {
            Some(input.int_in_range(0u8..=120)?)
        } else {
            None
        })
    };
    let finish = |input: &mut Unstructured| -> Result<Option<Finish>> {
        Ok(if input.arbitrary::<bool>()? {
            Some(*input.choose(&Finish::ALL)?)
        } else {
            None
        })
    };
    let mut faces = Vec::new();
    for _ in 0..input.int_in_range(0u8..=2)? {
        faces.push(FaceColour {
            face: scene.face(input, body)?,
            colour: colour(input)?,
            opacity: opacity(input)?,
            finish: finish(input)?,
        });
    }
    Ok(BodyAppearance {
        colour: if input.arbitrary::<bool>()? {
            Some(colour(input)?)
        } else {
            None
        },
        material: if input.arbitrary::<bool>()? {
            Some((*input.choose(&MATERIALS)?).to_owned())
        } else {
            None
        },
        density: if input.arbitrary::<bool>()? {
            scene.document.parse(input.choose(&DENSITIES)?).ok()
        } else {
            None
        },
        name: optional_text(input)?,
        opacity: opacity(input)?,
        finish: finish(input)?,
        faces,
    })
}

fn features(input: &mut Unstructured, scene: &Scene) -> Result<Vec<FeatureId>> {
    let mut chosen = Vec::new();
    for _ in 0..input.int_in_range(1u8..=3)? {
        chosen.push(scene.feature(input)?);
    }
    Ok(chosen)
}

pub fn tree_edit(input: &mut Unstructured, scene: &Scene) -> Result<Option<Transaction>> {
    let document = scene.document;
    let count = document.features().len();
    Ok(match input.int_in_range(0u8..=11)? {
        0 => single(Edit::RemoveFeature {
            id: scene.feature(input)?,
        }),
        1 => single(Edit::MoveFeature {
            id: scene.feature(input)?,
            index: input.int_in_range(0..=count)?,
        }),
        2 => single(Edit::RenameFeature {
            id: scene.feature(input)?,
            name: if input.arbitrary::<bool>()? {
                (*input.choose(&NAMES)?).to_owned()
            } else {
                text(input)?
            },
        }),
        3 => single(Edit::SetFeatureHidden {
            id: scene.feature(input)?,
            hidden: input.arbitrary()?,
        }),
        4 => single(Edit::SetFeatureSuppressed {
            id: scene.feature(input)?,
            suppressed: input.arbitrary()?,
        }),
        5 => single(Edit::SetRollbackBar {
            bar: if input.arbitrary::<bool>()? {
                RollbackBar::AtEnd
            } else {
                RollbackBar::Before(scene.feature(input)?)
            },
        }),
        6 => single(Edit::SetPrincipalHidden {
            geometry: *input.choose(&PrincipalGeometry::ALL)?,
            hidden: input.arbitrary()?,
        }),
        7 => single(Edit::SetFeatureGroup {
            id: scene.feature(input)?,
            group: optional_text(input)?,
        }),
        8 => {
            let members = features(input, scene)?;
            let built = if input.arbitrary::<bool>()? {
                document.grouping(&members, &text(input)?, LABEL)
            } else {
                document.regrouping(&members, optional_text(input)?, LABEL)
            };
            built.ok()
        }
        9 => {
            let body = scene.body(input)?;
            single(Edit::SetBodyAppearance {
                id: body,
                appearance: appearance(input, scene, body)?,
            })
        }
        _ => {
            let Some(feature) = chosen(input, &scene.feature_ids())? else {
                return Ok(None);
            };
            let Some(existing) = document.feature(feature) else {
                return Ok(None);
            };
            feature_kind(input, scene, Family::of(&existing.kind))?
                .and_then(|kind| single(Edit::SetFeatureKind { id: feature, kind }))
        }
    })
}

fn sketch_of<'a>(scene: &Scene<'a>, feature: FeatureId) -> Option<&'a Sketch> {
    scene.document.feature(feature)?.kind.sketch()
}

fn some_constraint(
    input: &mut Unstructured,
    scene: &Scene,
    feature: FeatureId,
) -> Result<ConstraintId> {
    let constraints: Vec<ConstraintId> = sketch_of(scene, feature)
        .map(|sketch| sketch.constraints().map(|(id, _)| id).collect())
        .unwrap_or_default();
    match chosen(input, &constraints)? {
        Some(id) if !input.ratio(1u8, 32)? => Ok(id),
        _ => Ok(ConstraintId::from_raw(input.int_in_range(0u64..=32)?)),
    }
}

fn projection_source(input: &mut Unstructured, scene: &Scene) -> Result<ProjectionSource> {
    let reach = *input.choose(&DISTANCES)?;
    Ok(match input.int_in_range(0u8..=5)? {
        0 => {
            let body = scene.body(input)?;
            ProjectionSource::Edge {
                body,
                edge: scene.edge(input, body)?,
            }
        }
        1 => {
            let body = scene.body(input)?;
            ProjectionSource::Vertex {
                body,
                vertex: scene.vertex(input, body)?,
            }
        }
        2 => {
            let sketch = scene.sketch(input)?;
            ProjectionSource::SketchEntity {
                sketch,
                entity: scene.sketch_entity(input, sketch)?,
            }
        }
        3 => {
            let body = scene.body(input)?;
            ProjectionSource::Section {
                body,
                edge: scene.edge(input, body)?,
            }
        }
        4 => ProjectionSource::DatumPlane {
            datum: scene.datum(input)?,
            reach,
        },
        _ => ProjectionSource::PrincipalPlane {
            plane: *input.choose(&PrincipalPlane::ALL)?,
            reach,
        },
    })
}

fn outline(input: &mut Unstructured) -> Result<Outline> {
    Ok(match input.int_in_range(0u8..=2)? {
        0 => Outline::Point(point(input)?),
        1 => Outline::Line {
            start: point(input)?,
            end: point(input)?,
        },
        _ => Outline::Circle {
            center: point(input)?,
            radius: length(input)?,
        },
    })
}

fn attachment(input: &mut Unstructured, scene: &Scene) -> Result<Option<SketchAttachment>> {
    Ok(match input.int_in_range(0u8..=4)? {
        0 => Some(SketchAttachment::Face(scene.face_attachment(input)?)),
        1 => Some(SketchAttachment::Datum(scene.datum(input)?)),
        2 => Some(SketchAttachment::Frame {
            frame: scene.datum(input)?,
            plane: *input.choose(&PrincipalPlane::ALL)?,
        }),
        _ => None,
    })
}

pub fn sketch_edit(input: &mut Unstructured, scene: &Scene) -> Result<Option<Transaction>> {
    let Some(feature) = scene.existing_sketch(input)? else {
        return Ok(None);
    };
    let Some(sketch) = sketch_of(scene, feature) else {
        return Ok(None);
    };
    let entities: Vec<EntityId> = sketch.entities().map(|(id, _)| id).collect();
    let points: Vec<EntityId> = sketch
        .entities()
        .filter(|(_, entity)| matches!(entity, Entity::Point(_)))
        .map(|(id, _)| id)
        .collect();
    let some_entity = |input: &mut Unstructured| -> Result<EntityId> {
        match chosen(input, &entities)? {
            Some(id) if !input.ratio(1u8, 32)? => Ok(id),
            _ => Ok(EntityId::from_raw(input.int_in_range(0u64..=32)?)),
        }
    };
    let new_entity = |input: &mut Unstructured| -> Result<Entity> {
        Ok(match (input.int_in_range(0u8..=2)?, points.as_slice()) {
            (1, [_, ..]) => Entity::Line {
                start: *input.choose(&points)?,
                end: *input.choose(&points)?,
            },
            (2, [_, ..]) => Entity::Circle {
                center: *input.choose(&points)?,
                radius: length(input)?,
            },
            _ => Entity::Point(point(input)?),
        })
    };
    let label = |input: &mut Unstructured| -> Result<Option<Vector2>> {
        Ok(if input.arbitrary::<bool>()? {
            Some(Vector2::new(coordinate(input)?, coordinate(input)?))
        } else {
            None
        })
    };
    Ok(match input.int_in_range(0u8..=12)? {
        0 => single(Edit::AddSketchEntity {
            feature,
            id: EntityId::from_raw(sketch.next_id()),
            entity: new_entity(input)?,
            construction: input.arbitrary()?,
        }),
        1 => single(Edit::RemoveSketchEntity {
            feature,
            id: some_entity(input)?,
        }),
        2 => single(Edit::SetSketchEntity {
            feature,
            id: some_entity(input)?,
            entity: new_entity(input)?,
        }),
        3 => single(Edit::SetSketchConstruction {
            feature,
            id: some_entity(input)?,
            construction: input.arbitrary()?,
        }),
        4 => single(Edit::AddSketchConstraint {
            feature,
            id: ConstraintId::from_raw(sketch.next_id()),
            constraint: constraint(input, sketch)?,
            inactive: input.arbitrary()?,
            label: label(input)?,
        }),
        5 => single(Edit::RemoveSketchConstraint {
            feature,
            id: some_constraint(input, scene, feature)?,
        }),
        6 => single(Edit::SetDimension {
            feature,
            constraint: some_constraint(input, scene, feature)?,
            value: if input.arbitrary::<bool>()? {
                scene.distance(input)?
            } else {
                length_value(input)?
            },
        }),
        7 => single(Edit::SetSketchConstraintActive {
            feature,
            id: some_constraint(input, scene, feature)?,
            active: input.arbitrary()?,
        }),
        8 => single(Edit::SetSketchLabel {
            feature,
            id: some_constraint(input, scene, feature)?,
            offset: label(input)?,
        }),
        9 => single(Edit::SetSketchProjection {
            feature,
            id: some_entity(input)?,
            source: if input.ratio(1u8, 4)? {
                None
            } else {
                Some(projection_source(input, scene)?)
            },
        }),
        10 => {
            let mut transaction = scene.document.transaction(LABEL);
            let source = projection_source(input, scene)?;
            transaction.add_projection(feature, source, &outline(input)?);
            Some(transaction.finish())
        }
        11 => {
            let mut transaction = scene.document.transaction(LABEL);
            let removed = [some_entity(input)?];
            let constraints = [some_constraint(input, scene, feature)?];
            transaction.remove_sketch_items(feature, removed, constraints);
            Some(transaction.finish())
        }
        _ => single(Edit::SetSketchPlacement {
            feature,
            plane: plane(input)?,
            attachment: attachment(input, scene)?,
        }),
    })
}

fn saved_view(input: &mut Unstructured) -> Result<SavedView> {
    let component = |input: &mut Unstructured| -> Result<f64> {
        Ok(f64::from(input.int_in_range(-2i8..=2)?) * 0.5)
    };
    Ok(SavedView {
        target: Point3::new(coordinate(input)?, coordinate(input)?, coordinate(input)?),
        orientation: Rotation3::from_xyzw(
            component(input)?,
            component(input)?,
            component(input)?,
            component(input)?,
        ),
        distance: *input.choose(&DISTANCES)?,
    })
}

fn properties(input: &mut Unstructured, scene: &Scene) -> Result<ModelProperties> {
    let mut properties = scene.document.properties().clone();
    for _ in 0..input.int_in_range(1u8..=3)? {
        let property = *input.choose(&ModelProperty::ALL)?;
        *properties.get_mut(property) = text(input)?;
    }
    Ok(properties)
}

fn views(input: &mut Unstructured, scene: &Scene) -> Result<SavedViews> {
    let mut views = scene.document.saved_views().clone();
    match input.int_in_range(0u8..=3)? {
        0 => views.named.push(NamedView {
            name: text(input)?,
            view: saved_view(input)?,
        }),
        1 if !views.named.is_empty() => {
            let index = input.choose_index(views.named.len())?;
            views.named.remove(index);
        }
        2 => {
            views.home = if input.arbitrary::<bool>()? {
                Some(saved_view(input)?)
            } else {
                None
            }
        }
        _ => {
            if let Some(named) = views.named.first_mut() {
                named.name = text(input)?;
            }
        }
    }
    Ok(views)
}

fn member(input: &mut Unstructured, scene: &Scene) -> Result<SetMember> {
    let body = scene.body(input)?;
    Ok(match input.int_in_range(0u8..=2)? {
        0 => SetMember::Body(body),
        1 => SetMember::Face {
            body,
            face: scene.face(input, body)?,
        },
        _ => SetMember::Edge {
            body,
            edge: scene.edge(input, body)?,
        },
    })
}

fn selection_sets(input: &mut Unstructured, scene: &Scene) -> Result<SelectionSets> {
    let mut sets = scene.document.selection_sets().clone();
    match input.int_in_range(0u8..=2)? {
        0 => {
            let mut members = Vec::new();
            for _ in 0..input.int_in_range(0..=MOST_MEMBERS)? {
                members.push(member(input, scene)?);
            }
            sets.sets.push(SelectionSet {
                name: text(input)?,
                members,
            });
        }
        1 if !sets.sets.is_empty() => {
            let index = input.choose_index(sets.sets.len())?;
            sets.sets.remove(index);
        }
        _ => {
            if let Some(set) = sets.sets.first_mut() {
                set.name = text(input)?;
                set.members.push(member(input, scene)?);
            }
        }
    }
    Ok(sets)
}

fn configured_value(input: &mut Unstructured, scene: &Scene) -> Result<ConfiguredValue> {
    Ok(
        match (input.int_in_range(0u8..=2)?, scene.parameter(input)?) {
            (0, Some(parameter)) => ConfiguredValue::Parameter(parameter),
            (1, _) => ConfiguredValue::Suppressed(scene.feature(input)?),
            _ => ConfiguredValue::Colour(scene.body(input)?),
        },
    )
}

fn setting(input: &mut Unstructured, scene: &Scene, value: ConfiguredValue) -> Result<Setting> {
    Ok(match value {
        ConfiguredValue::Parameter(_) => Setting::Expression(scene.distance(input)?),
        ConfiguredValue::Suppressed(_) => Setting::Suppressed(input.arbitrary()?),
        ConfiguredValue::Colour(_) => Setting::Colour(if input.arbitrary::<bool>()? {
            Some(Rgb {
                red: input.arbitrary()?,
                green: input.arbitrary()?,
                blue: input.arbitrary()?,
            })
        } else {
            None
        }),
    })
}

fn configuration_id(input: &mut Unstructured, scene: &Scene) -> Result<ConfigurationId> {
    let ids: Vec<ConfigurationId> = scene
        .document
        .configurations()
        .rows
        .iter()
        .map(|row| row.id)
        .collect();
    match chosen(input, &ids)? {
        Some(id) if !input.ratio(1u8, 16)? => Ok(id),
        _ => Ok(ConfigurationId::from_raw(input.int_in_range(0u64..=8)?)),
    }
}

fn configurations(input: &mut Unstructured, scene: &Scene) -> Result<Configurations> {
    let mut values = Vec::new();
    for _ in 0..input.int_in_range(0u8..=2)? {
        values.push(configured_value(input, scene)?);
    }
    let mut rows = Vec::new();
    for _ in 0..input.int_in_range(0..=MOST_ROWS)? {
        let mut settings = BTreeMap::new();
        for value in &values {
            if input.ratio(3u8, 4)? {
                settings.insert(*value, setting(input, scene, *value)?);
            }
        }
        rows.push(Configuration {
            id: ConfigurationId::from_raw(input.int_in_range(0u64..=4)?),
            name: text(input)?,
            settings,
        });
    }
    let active = if input.arbitrary::<bool>()? {
        chosen(input, &rows)?.map(|row| row.id)
    } else {
        None
    };
    Ok(Configurations {
        values,
        rows,
        active,
        next_id: input.int_in_range(0u64..=8)?,
    })
}

pub fn model_edit(input: &mut Unstructured, scene: &Scene) -> Result<Option<Transaction>> {
    let document = scene.document;
    Ok(match input.int_in_range(0u8..=14)? {
        0 => single(Edit::SetModelProperties {
            properties: Box::new(properties(input, scene)?),
        }),
        1 => single(Edit::SetSavedViews {
            views: Box::new(views(input, scene)?),
        }),
        2 => single(Edit::SetSelectionSets {
            sets: Box::new(selection_sets(input, scene)?),
        }),
        3 => single(Edit::SetConfigurations {
            configurations: Box::new(configurations(input, scene)?),
        }),
        4 => single(Edit::SetActiveConfiguration {
            active: if input.ratio(1u8, 4)? {
                None
            } else {
                Some(configuration_id(input, scene)?)
            },
        }),
        5 => {
            let name = optional_text(input)?;
            Some(document.adding_configuration(name.as_deref()).0)
        }
        6 => document
            .duplicating_configuration(configuration_id(input, scene)?)
            .ok()
            .map(|(transaction, _)| transaction),
        7 => document
            .renaming_configuration(configuration_id(input, scene)?, &text(input)?)
            .ok(),
        8 => {
            let rows = document.configurations().rows.len();
            document
                .moving_configuration(
                    configuration_id(input, scene)?,
                    input.int_in_range(0..=rows)?,
                )
                .ok()
        }
        9 => document
            .deleting_configuration(configuration_id(input, scene)?)
            .ok(),
        10 => document.configuring(configured_value(input, scene)?).ok(),
        11 => Some(document.unconfiguring(configured_value(input, scene)?)),
        12 => {
            let value = configured_value(input, scene)?;
            let setting = setting(input, scene, value)?;
            document
                .setting_configuration(configuration_id(input, scene)?, value, setting)
                .ok()
        }
        _ => document.activating(configuration_id(input, scene)?).ok(),
    })
}
