use caditor_document::{
    AxisReference, AxisSide, BodyOperation, CancelToken, Datum, DatumAxis, DatumPlane, DatumPoint, Document,
    Edit, Editor, Evaluation, Extrude, ExtrudeEnd, ExtrudeExtent, Feature, FeatureId, FeatureKind,
    ModelEvaluator, PlaneReference, PlaneThrough, PointReference, PrincipalAxis, PrincipalGeometry,
    PrincipalPlane, Recompute, RegionChoice, Revolve, RevolveAxis, RevolveExtent, RollbackBar,
    SolidFeature, Transaction,
};
use caditor_expression::{Expression, ParameterId};
use caditor_geometry::Plane;
use caditor_sketch::{ConstraintId, Entity, EntityId, Sketch};
use libfuzzer_sys::arbitrary::{Result, Unstructured};

use crate::{
    WORK_BUDGET, interrupt_after, plane, point,
    sketch::{constraint, length_value, sketch},
};

const MOST_STEPS: usize = 24;
const NAMES: [&str; 6] = ["width", "depth", "angle", "a", "b", "Sketch"];
const EXPRESSIONS: [&str; 10] = [
    "5 mm",
    "width / 2",
    "depth + 1 mm",
    "30 deg",
    "a * b",
    "2",
    "angle",
    "width",
    "-3 mm",
    "sqrt(width * depth)",
];

fn feature_ids(document: &Document) -> Vec<FeatureId> {
    document.features().map(Feature::id).collect()
}

fn feature(input: &mut Unstructured, document: &Document) -> Result<FeatureId> {
    let ids = feature_ids(document);
    if ids.is_empty() || input.ratio(1u8, 32)? {
        return Ok(FeatureId::from_raw(input.int_in_range(0..=64)?));
    }
    Ok(*input.choose(&ids)?)
}

fn sketch_feature(input: &mut Unstructured, document: &Document) -> Result<Option<FeatureId>> {
    let sketches: Vec<FeatureId> = document
        .features()
        .filter(|feature| feature.kind.sketch().is_some())
        .map(Feature::id)
        .collect();
    if sketches.is_empty() {
        return Ok(None);
    }
    Ok(Some(*input.choose(&sketches)?))
}

fn parameter(input: &mut Unstructured, document: &Document) -> Result<Option<ParameterId>> {
    let ids: Vec<ParameterId> = document.parameters().iter().map(|p| p.id()).collect();
    if ids.is_empty() {
        return Ok(None);
    }
    Ok(Some(*input.choose(&ids)?))
}

fn expression(input: &mut Unstructured, document: &Document) -> Result<Option<Expression>> {
    Ok(document.parse(input.choose(&EXPRESSIONS)?).ok())
}

fn plane_reference(input: &mut Unstructured, document: &Document) -> Result<PlaneReference> {
    Ok(if input.arbitrary::<bool>()? {
        PlaneReference::Principal(*input.choose(&PrincipalPlane::ALL)?)
    } else {
        PlaneReference::Datum(feature(input, document)?)
    })
}

fn axis_reference(input: &mut Unstructured, document: &Document) -> Result<AxisReference> {
    Ok(if input.arbitrary::<bool>()? {
        AxisReference::Principal(*input.choose(&PrincipalAxis::ALL)?)
    } else {
        AxisReference::Datum(feature(input, document)?)
    })
}

fn point_reference(input: &mut Unstructured, document: &Document) -> Result<PointReference> {
    Ok(match input.int_in_range(0u8..=2)? {
        0 => PointReference::Origin,
        1 => PointReference::Datum(feature(input, document)?),
        _ => PointReference::Sketch {
            sketch: feature(input, document)?,
            entity: EntityId::from_raw(input.int_in_range(0u64..=16)?),
        },
    })
}

fn operation(input: &mut Unstructured, document: &Document) -> Result<BodyOperation> {
    Ok(match input.int_in_range(0u8..=3)? {
        0 => BodyOperation::NewBody,
        1 => BodyOperation::Add(feature(input, document)?),
        2 => BodyOperation::Remove(feature(input, document)?),
        _ => BodyOperation::Intersect(feature(input, document)?),
    })
}

fn extrude_end(input: &mut Unstructured, document: &Document) -> Result<ExtrudeEnd> {
    Ok(match input.int_in_range(0u8..=3)? {
        0 => ExtrudeEnd::ThroughAll,
        1 => ExtrudeEnd::UpToNext,
        2 => ExtrudeEnd::UpToFace(plane_reference(input, document)?),
        _ => ExtrudeEnd::Distance(distance(input, document)?),
    })
}

fn distance(input: &mut Unstructured, document: &Document) -> Result<Expression> {
    match expression(input, document)? {
        Some(expression) if input.arbitrary::<bool>()? => Ok(expression),
        _ => length_value(input),
    }
}

fn turn(input: &mut Unstructured, document: &Document) -> Result<Expression> {
    match expression(input, document)? {
        Some(expression) if input.arbitrary::<bool>()? => Ok(expression),
        _ => crate::sketch::angle_value(input),
    }
}

fn regions(input: &mut Unstructured) -> Result<RegionChoice> {
    Ok(if input.ratio(1u8, 8)? {
        RegionChoice::Chosen(Vec::new())
    } else {
        RegionChoice::All
    })
}

fn sketch_lines(document: &Document, sketch: FeatureId) -> Vec<EntityId> {
    document
        .feature(sketch)
        .and_then(|feature| feature.kind.sketch())
        .map(|sketch| {
            sketch
                .entities()
                .filter(|(_, entity)| matches!(entity, Entity::Line { .. }))
                .map(|(id, _)| id)
                .collect()
        })
        .unwrap_or_default()
}

fn solid_feature(input: &mut Unstructured, document: &Document) -> Result<Option<FeatureKind>> {
    let Some(sketch) = sketch_feature(input, document)? else {
        return Ok(None);
    };
    let regions = regions(input)?;
    let operation = operation(input, document)?;
    let solid = if input.arbitrary::<bool>()? {
        let extent = match input.int_in_range(0u8..=2)? {
            0 => ExtrudeExtent::OneSide {
                end: extrude_end(input, document)?,
                reversed: input.arbitrary()?,
            },
            1 => ExtrudeExtent::Symmetric {
                distance: distance(input, document)?,
            },
            _ => ExtrudeExtent::TwoSides {
                forward: extrude_end(input, document)?,
                backward: extrude_end(input, document)?,
            },
        };
        SolidFeature::Extrude(Extrude {
            sketch,
            regions,
            extent,
            operation,
            start: None,
            other_bodies: Vec::new(),
        })
    } else {
        let lines = sketch_lines(document, sketch);
        let axis = if !lines.is_empty() && input.arbitrary::<bool>()? {
            RevolveAxis::Sketch(*input.choose(&lines)?)
        } else if input.arbitrary::<bool>()? {
            RevolveAxis::Sketch(*input.choose(&EntityId::REFERENCES)?)
        } else {
            RevolveAxis::Model(axis_reference(input, document)?)
        };
        let extent = match input.int_in_range(0u8..=3)? {
            0 => RevolveExtent::Full,
            1 => RevolveExtent::OneSide {
                angle: turn(input, document)?,
                reversed: input.arbitrary()?,
            },
            2 => RevolveExtent::Symmetric {
                angle: turn(input, document)?,
            },
            _ => RevolveExtent::TwoSides {
                forward: turn(input, document)?,
                backward: turn(input, document)?,
            },
        };
        SolidFeature::Revolve(Revolve {
            sketch,
            regions,
            axis,
            extent,
            operation,
            start: None,
            other_bodies: Vec::new(),
            side: match input.int_in_range(0u8..=2)? {
                0 => None,
                1 => Some(AxisSide::Left),
                _ => Some(AxisSide::Right),
            },
        })
    };
    Ok(Some(FeatureKind::Solid(solid)))
}

fn datum(input: &mut Unstructured, document: &Document) -> Result<FeatureKind> {
    Ok(FeatureKind::Datum(match input.int_in_range(0u8..=8)? {
        0 => Datum::Plane(DatumPlane {
            base: plane_reference(input, document)?,
            rotation: None,
            offset: distance(input, document)?,
        }),
        1 => Datum::Axis(DatumAxis::Along(axis_reference(input, document)?)),
        2 => Datum::Axis(DatumAxis::Intersection(
            plane_reference(input, document)?,
            plane_reference(input, document)?,
        )),
        3 => Datum::Axis(DatumAxis::Points(
            point_reference(input, document)?,
            point_reference(input, document)?,
        )),
        4 => Datum::Axis(DatumAxis::NormalTo(
            plane_reference(input, document)?,
            point_reference(input, document)?,
        )),
        5 => Datum::Point(DatumPoint {
            base: point_reference(input, document)?,
            offset: [
                distance(input, document)?,
                distance(input, document)?,
                distance(input, document)?,
            ],
        }),
        6 => Datum::PlaneThrough(PlaneThrough::Points([
            point_reference(input, document)?,
            point_reference(input, document)?,
            point_reference(input, document)?,
        ])),
        7 => Datum::PlaneThrough(PlaneThrough::Midway(
            plane_reference(input, document)?,
            plane_reference(input, document)?,
        )),
        _ => Datum::PlaneThrough(PlaneThrough::AxisAndPoint(
            axis_reference(input, document)?,
            point_reference(input, document)?,
        )),
    }))
}

fn sketch_of(document: &Document, feature: FeatureId) -> Option<&Sketch> {
    document.feature(feature)?.kind.sketch()
}

fn sketch_edit(input: &mut Unstructured, document: &Document) -> Result<Option<Edit>> {
    let Some(feature) = sketch_feature(input, document)? else {
        return Ok(None);
    };
    let Some(sketch) = sketch_of(document, feature) else {
        return Ok(None);
    };
    let entities: Vec<EntityId> = sketch.entities().map(|(id, _)| id).collect();
    let constraints: Vec<ConstraintId> = sketch.constraints().map(|(id, _)| id).collect();
    let some_entity = |input: &mut Unstructured| -> Result<EntityId> {
        if entities.is_empty() {
            Ok(EntityId::from_raw(input.arbitrary()?))
        } else {
            Ok(*input.choose(&entities)?)
        }
    };
    let some_constraint = |input: &mut Unstructured| -> Result<ConstraintId> {
        if constraints.is_empty() {
            Ok(ConstraintId::from_raw(input.arbitrary()?))
        } else {
            Ok(*input.choose(&constraints)?)
        }
    };
    Ok(Some(match input.int_in_range(0u8..=6)? {
        0 => Edit::AddSketchEntity {
            feature,
            id: EntityId::from_raw(sketch.next_id()),
            entity: Entity::Point(point(input)?),
            construction: false,
        },
        1 => Edit::RemoveSketchEntity {
            feature,
            id: some_entity(input)?,
        },
        2 => Edit::SetSketchEntity {
            feature,
            id: some_entity(input)?,
            entity: Entity::Point(point(input)?),
        },
        3 => Edit::SetSketchConstruction {
            feature,
            id: some_entity(input)?,
            construction: input.arbitrary()?,
        },
        4 => Edit::AddSketchConstraint {
            feature,
            id: ConstraintId::from_raw(sketch.next_id()),
            constraint: constraint(input, sketch)?,
            inactive: input.arbitrary()?,
        },
        5 => Edit::RemoveSketchConstraint {
            feature,
            id: some_constraint(input)?,
        },
        _ => Edit::SetDimension {
            feature,
            constraint: some_constraint(input)?,
            value: distance(input, document)?,
        },
    }))
}

fn tree_edit(input: &mut Unstructured, document: &Document) -> Result<Option<Edit>> {
    let count = document.features().len();
    Ok(Some(match input.int_in_range(0u8..=10)? {
        0 => Edit::RemoveFeature {
            id: feature(input, document)?,
        },
        1 => Edit::MoveFeature {
            id: feature(input, document)?,
            index: input.int_in_range(0..=count)?,
        },
        2 => Edit::RenameFeature {
            id: feature(input, document)?,
            name: (*input.choose(&NAMES)?).to_owned(),
        },
        3 => Edit::SetFeatureHidden {
            id: feature(input, document)?,
            hidden: input.arbitrary()?,
        },
        4 => Edit::SetFeatureSuppressed {
            id: feature(input, document)?,
            suppressed: input.arbitrary()?,
        },
        5 => Edit::SetRollbackBar {
            bar: if input.arbitrary::<bool>()? {
                RollbackBar::AtEnd
            } else {
                RollbackBar::Before(feature(input, document)?)
            },
        },
        6 => Edit::SetPrincipalHidden {
            geometry: *input.choose(&PrincipalGeometry::ALL)?,
            hidden: input.arbitrary()?,
        },
        7 => {
            let Some(id) = parameter(input, document)? else {
                return Ok(None);
            };
            Edit::RemoveParameter { id }
        }
        8 => {
            let Some(id) = parameter(input, document)? else {
                return Ok(None);
            };
            Edit::RenameParameter {
                id,
                name: (*input.choose(&NAMES)?).to_owned(),
            }
        }
        9 => {
            let (Some(id), Some(expression)) =
                (parameter(input, document)?, expression(input, document)?)
            else {
                return Ok(None);
            };
            Edit::SetParameterExpression { id, expression }
        }
        _ => {
            let Some(feature) = sketch_feature(input, document)? else {
                return Ok(None);
            };
            Edit::SetSketchPlacement {
                feature,
                plane: plane(input)?,
                attachment: None,
            }
        }
    }))
}

fn addition(input: &mut Unstructured, document: &Document) -> Result<Option<Transaction>> {
    let mut transaction = document.transaction("Fuzzed");
    match input.int_in_range(0u8..=3)? {
        0 => {
            let name = *input.choose(&NAMES)?;
            let Ok(expression) = transaction.parse(input.choose(&EXPRESSIONS)?) else {
                return Ok(None);
            };
            transaction.add_parameter(name, expression);
        }
        1 => {
            let plane: Plane = plane(input)?;
            let drawn = sketch(input, plane)?;
            transaction.add_feature(*input.choose(&NAMES)?, FeatureKind::from(drawn));
        }
        2 => {
            let Some(kind) = solid_feature(input, document)? else {
                return Ok(None);
            };
            transaction.add_feature(*input.choose(&NAMES)?, kind);
        }
        _ => {
            let kind = datum(input, document)?;
            transaction.add_feature(*input.choose(&NAMES)?, kind);
        }
    }
    Ok(Some(transaction.finish()))
}

pub enum Step {
    Apply(Transaction),
    Undo,
    Redo,
}

pub fn step(input: &mut Unstructured, document: &Document) -> Result<Option<Step>> {
    let single =
        |edit: Option<Edit>| edit.map(|edit| Step::Apply(Transaction::single("Fuzzed", edit)));
    Ok(match input.int_in_range(0u8..=9)? {
        0..=3 => addition(input, document)?.map(Step::Apply),
        4 | 5 => single(sketch_edit(input, document)?),
        6 | 7 => single(tree_edit(input, document)?),
        8 => Some(Step::Undo),
        _ => Some(Step::Redo),
    })
}

pub fn check_inverse(document: &Document, transaction: &Transaction) {
    let mut applied = document.clone();
    match applied.apply(transaction.clone()) {
        Ok(inverse) => {
            let mut restored = applied.clone();
            if let Err(error) = restored.apply(inverse) {
                panic!("the inverse of a change could not be applied: {error}");
            }
            assert!(
                restored.same_content(document),
                "undoing a change did not restore the document"
            );
            let mut rebuilt = document.clone();
            if let Err(error) = rebuilt.apply(document.transaction_to(&applied, "Restore")) {
                panic!("a transaction between two states could not be applied: {error}");
            }
            assert!(
                rebuilt.same_content(&applied),
                "a transaction between two states did not reach the target"
            );
        }
        Err(_) => assert!(
            applied == *document,
            "a refused change still changed the document"
        ),
    }
}

pub fn run(input: &mut Unstructured, editor: &mut Editor, mut visit: impl FnMut(&Editor)) {
    for _ in 0..MOST_STEPS {
        if input.is_empty() {
            break;
        }
        let Ok(Some(step)) = step(input, editor.document()) else {
            continue;
        };
        match step {
            Step::Apply(transaction) => {
                check_inverse(editor.document(), &transaction);
                let _ = editor.apply(transaction);
            }
            Step::Undo => {
                if let Err(error) = editor.undo() {
                    panic!("undo failed: {error}");
                }
            }
            Step::Redo => {
                if let Err(error) = editor.redo() {
                    panic!("redo failed: {error}");
                }
            }
        }
        visit(editor);
    }
}

pub fn recompute(document: &Document) -> Evaluation {
    let interrupt = interrupt_after(WORK_BUDGET);
    let cancel = CancelToken::new(move || interrupt());
    Recompute::default().run(document, &ModelEvaluator, &cancel, &|_, _| {})
}
