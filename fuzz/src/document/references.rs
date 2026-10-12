use std::collections::BTreeMap;

use caditor_document::{
    AxisReference, Document, Evaluation, FaceAttachment, Feature, FeatureId, PlaneReference,
    PointReference, PrincipalAxis, PrincipalPlane,
};
use caditor_expression::{Expression, ParameterId};
use caditor_kernel::{
    EdgeName, EdgeNaming, EdgeReference, FaceCopy, FaceName, FaceOrigin, FaceReference, Made,
    VertexName,
};
use caditor_sketch::{Entity, EntityId};
use libfuzzer_sys::arbitrary::{Result, Unstructured};

use crate::sketch::{angle_value, length_value};

const MOST_CAPTURED: usize = 48;
const SYNTHETIC_NAMES: u8 = 8;
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
const COUNTS: [f64; 6] = [1.0, 2.0, 3.0, 0.0, -1.0, 2.5];
const FACTORS: [f64; 6] = [1.0, 2.0, 0.5, 0.0, -1.0, 1e9];

#[derive(Debug, Default)]
pub struct References {
    faces: BTreeMap<FeatureId, Vec<FaceReference>>,
    edges: BTreeMap<FeatureId, Vec<EdgeReference>>,
}

impl References {
    pub fn capture(evaluation: &Evaluation) -> Self {
        let mut references = Self::default();
        for (body, _) in evaluation.bodies() {
            let Some(solid) = evaluation.body(body) else {
                continue;
            };
            let faces = solid
                .faces()
                .take(MOST_CAPTURED)
                .filter_map(|(face, _)| FaceReference::capture(solid, face))
                .collect();
            let naming = EdgeNaming::new(solid);
            let edges = solid
                .edges()
                .take(MOST_CAPTURED)
                .filter_map(|(edge, _)| EdgeReference::capture_in(&naming, edge))
                .collect();
            references.faces.insert(body, faces);
            references.edges.insert(body, edges);
        }
        references
    }
}

pub struct Scene<'a> {
    pub document: &'a Document,
    pub references: &'a References,
}

impl Scene<'_> {
    pub fn feature_ids(&self) -> Vec<FeatureId> {
        self.document.features().map(Feature::id).collect()
    }

    pub fn feature(&self, input: &mut Unstructured) -> Result<FeatureId> {
        let ids = self.feature_ids();
        if ids.is_empty() || input.ratio(1u8, 32)? {
            return Ok(FeatureId::from_raw(input.int_in_range(0..=64)?));
        }
        Ok(*input.choose(&ids)?)
    }

    fn matching(&self, keep: impl Fn(&Feature) -> bool) -> Vec<FeatureId> {
        self.document
            .features()
            .filter(|feature| keep(feature))
            .map(Feature::id)
            .collect()
    }

    fn one_of(
        &self,
        input: &mut Unstructured,
        keep: impl Fn(&Feature) -> bool,
    ) -> Result<FeatureId> {
        let matching = self.matching(keep);
        if matching.is_empty() || input.ratio(1u8, 16)? {
            return self.feature(input);
        }
        Ok(*input.choose(&matching)?)
    }

    pub fn body(&self, input: &mut Unstructured) -> Result<FeatureId> {
        self.one_of(input, Feature::makes_body)
    }

    pub fn sketch(&self, input: &mut Unstructured) -> Result<FeatureId> {
        self.one_of(input, |feature| feature.kind.sketch().is_some())
    }

    pub fn datum(&self, input: &mut Unstructured) -> Result<FeatureId> {
        self.one_of(input, |feature| feature.kind.datum().is_some())
    }

    pub fn existing_sketch(&self, input: &mut Unstructured) -> Result<Option<FeatureId>> {
        let sketches = self.matching(|feature| feature.kind.sketch().is_some());
        if sketches.is_empty() {
            return Ok(None);
        }
        Ok(Some(*input.choose(&sketches)?))
    }

    pub fn bodies(&self, input: &mut Unstructured) -> Result<Vec<FeatureId>> {
        let mut bodies = Vec::new();
        for _ in 0..input.int_in_range(0u8..=2)? {
            bodies.push(self.body(input)?);
        }
        Ok(bodies)
    }

    pub fn parameter(&self, input: &mut Unstructured) -> Result<Option<ParameterId>> {
        let ids: Vec<ParameterId> = self
            .document
            .parameters()
            .iter()
            .map(|parameter| parameter.id())
            .collect();
        if ids.is_empty() {
            return Ok(None);
        }
        Ok(Some(*input.choose(&ids)?))
    }

    pub fn sketch_entity(&self, input: &mut Unstructured, sketch: FeatureId) -> Result<EntityId> {
        let mut entities: Vec<EntityId> = self
            .document
            .feature(sketch)
            .and_then(|feature| feature.kind.sketch())
            .map(|sketch| sketch.entities().map(|(id, _)| id).collect())
            .unwrap_or_default();
        entities.extend(EntityId::REFERENCES);
        if input.ratio(1u8, 32)? {
            return Ok(EntityId::from_raw(input.int_in_range(0u64..=16)?));
        }
        Ok(*input.choose(&entities)?)
    }

    pub fn sketch_lines(&self, sketch: FeatureId) -> Vec<EntityId> {
        self.document
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

    pub fn expression(&self, input: &mut Unstructured) -> Result<Option<Expression>> {
        Ok(self.document.parse(input.choose(&EXPRESSIONS)?).ok())
    }

    pub fn distance(&self, input: &mut Unstructured) -> Result<Expression> {
        match self.expression(input)? {
            Some(expression) if input.arbitrary::<bool>()? => Ok(expression),
            _ => length_value(input),
        }
    }

    pub fn turn(&self, input: &mut Unstructured) -> Result<Expression> {
        match self.expression(input)? {
            Some(expression) if input.arbitrary::<bool>()? => Ok(expression),
            _ => angle_value(input),
        }
    }

    pub fn count(&self, input: &mut Unstructured) -> Result<Expression> {
        match self.expression(input)? {
            Some(expression) if input.ratio(1u8, 4)? => Ok(expression),
            _ => Ok(Expression::number(*input.choose(&COUNTS)?)),
        }
    }

    pub fn factor(&self, input: &mut Unstructured) -> Result<Expression> {
        match self.expression(input)? {
            Some(expression) if input.ratio(1u8, 4)? => Ok(expression),
            _ => Ok(Expression::number(*input.choose(&FACTORS)?)),
        }
    }

    fn synthetic_face_name(input: &mut Unstructured) -> Result<FaceName> {
        Ok(FaceName::from_digest(u128::from(
            input.int_in_range(0..=SYNTHETIC_NAMES)?,
        )))
    }

    fn face_origin(&self, input: &mut Unstructured) -> Result<Option<FaceOrigin>> {
        let feature = self.feature(input)?.raw();
        let entity = input.int_in_range(0u64..=16)?;
        let face = input.int_in_range(0u32..=8)?;
        Ok(Some(match input.int_in_range(0u8..=8)? {
            0 => return Ok(None),
            1 => FaceOrigin::Side { feature, entity },
            2 => FaceOrigin::StartCap { feature },
            3 => FaceOrigin::EndCap { feature },
            4 => FaceOrigin::Fillet { feature },
            5 => FaceOrigin::Chamfer { feature },
            6 => FaceOrigin::Shell { feature },
            7 => FaceOrigin::Imported { feature, face },
            _ => FaceOrigin::Copy {
                copy: FaceCopy {
                    pattern: self.feature(input)?.raw(),
                    index: [input.int_in_range(0u32..=3)?, input.int_in_range(0u32..=3)?],
                },
                feature,
                made: match input.int_in_range(0u8..=6)? {
                    0 => Made::Side { entity },
                    1 => Made::StartCap,
                    2 => Made::EndCap,
                    3 => Made::Fillet,
                    4 => Made::Chamfer,
                    5 => Made::Shell,
                    _ => Made::Imported { face },
                },
            },
        }))
    }

    pub fn face(&self, input: &mut Unstructured, body: FeatureId) -> Result<FaceReference> {
        if let Some(captured) = self.references.faces.get(&body)
            && !captured.is_empty()
            && !input.ratio(1u8, 8)?
        {
            return Ok(input.choose(captured)?.clone());
        }
        let mut neighbours = Vec::new();
        for _ in 0..input.int_in_range(0u8..=2)? {
            neighbours.push(Self::synthetic_face_name(input)?);
        }
        Ok(FaceReference::new(
            Self::synthetic_face_name(input)?,
            self.face_origin(input)?,
            neighbours,
        ))
    }

    pub fn faces(&self, input: &mut Unstructured, body: FeatureId) -> Result<Vec<FaceReference>> {
        let mut faces = Vec::new();
        for _ in 0..input.int_in_range(0u8..=3)? {
            faces.push(self.face(input, body)?);
        }
        Ok(faces)
    }

    pub fn edge(&self, input: &mut Unstructured, body: FeatureId) -> Result<EdgeReference> {
        if let Some(captured) = self.references.edges.get(&body)
            && !captured.is_empty()
            && !input.ratio(1u8, 8)?
        {
            return Ok(*input.choose(captured)?);
        }
        let name = |input: &mut Unstructured| -> Result<u128> {
            Ok(u128::from(input.int_in_range(0..=SYNTHETIC_NAMES)?))
        };
        let edge = EdgeReference::new(
            EdgeName::from_digest(name(input)?),
            [
                Self::synthetic_face_name(input)?,
                Self::synthetic_face_name(input)?,
            ],
            [
                VertexName::from_digest(name(input)?),
                VertexName::from_digest(name(input)?),
            ],
        );
        Ok(edge.with_origins([self.face_origin(input)?, self.face_origin(input)?]))
    }

    pub fn edges(&self, input: &mut Unstructured, body: FeatureId) -> Result<Vec<EdgeReference>> {
        let mut edges = Vec::new();
        for _ in 0..input.int_in_range(1u8..=3)? {
            edges.push(self.edge(input, body)?);
        }
        Ok(edges)
    }

    pub fn vertex(&self, input: &mut Unstructured, body: FeatureId) -> Result<VertexName> {
        let ends = self.edge(input, body)?.ends();
        Ok(*input.choose(&ends)?)
    }

    pub fn face_attachment(&self, input: &mut Unstructured) -> Result<FaceAttachment> {
        let body = self.body(input)?;
        Ok(FaceAttachment {
            body,
            face: self.face(input, body)?,
        })
    }

    pub fn plane(&self, input: &mut Unstructured) -> Result<PlaneReference> {
        Ok(match input.int_in_range(0u8..=5)? {
            0 | 1 => PlaneReference::Principal(*input.choose(&PrincipalPlane::ALL)?),
            2 | 3 => PlaneReference::Datum(self.datum(input)?),
            4 => PlaneReference::Face(self.face_attachment(input)?),
            _ => PlaneReference::Frame {
                frame: self.datum(input)?,
                plane: *input.choose(&PrincipalPlane::ALL)?,
            },
        })
    }

    pub fn axis(&self, input: &mut Unstructured) -> Result<AxisReference> {
        Ok(match input.int_in_range(0u8..=6)? {
            0 | 1 => AxisReference::Principal(*input.choose(&PrincipalAxis::ALL)?),
            2 => AxisReference::Datum(self.datum(input)?),
            3 => {
                let body = self.body(input)?;
                AxisReference::Edge {
                    body,
                    edge: Box::new(self.edge(input, body)?),
                }
            }
            4 => {
                let body = self.body(input)?;
                AxisReference::Face {
                    body,
                    face: self.face(input, body)?,
                }
            }
            5 => {
                let sketch = self.sketch(input)?;
                AxisReference::Sketch {
                    sketch,
                    entity: self.sketch_entity(input, sketch)?,
                }
            }
            _ => AxisReference::Frame {
                frame: self.datum(input)?,
                axis: *input.choose(&PrincipalAxis::ALL)?,
            },
        })
    }

    pub fn point(&self, input: &mut Unstructured) -> Result<PointReference> {
        Ok(match input.int_in_range(0u8..=6)? {
            0 => PointReference::Origin,
            1 => PointReference::Datum(self.datum(input)?),
            2 => {
                let body = self.body(input)?;
                PointReference::Vertex {
                    body,
                    vertex: self.vertex(input, body)?,
                }
            }
            3 => {
                let body = self.body(input)?;
                PointReference::Centre {
                    body,
                    edge: Box::new(self.edge(input, body)?),
                }
            }
            4 => {
                let body = self.body(input)?;
                PointReference::SurfaceCentre {
                    body,
                    face: self.face(input, body)?,
                }
            }
            5 => {
                let sketch = self.sketch(input)?;
                PointReference::Sketch {
                    sketch,
                    entity: self.sketch_entity(input, sketch)?,
                }
            }
            _ => PointReference::Frame(self.datum(input)?),
        })
    }
}
