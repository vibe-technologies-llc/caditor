use std::{
    collections::BTreeSet,
    sync::{Arc, OnceLock},
    time::SystemTime,
};

use caditor_document::{
    AngleMate, AngleSides, AxisMate, AxisSide, AxisTurn, Between, Blend, BlendKind, BodyOperation,
    BodyPlacement, Calibration, ChamferForm, CircularPattern, Combine, CombineOperation,
    CopyOrientation, CurvePattern, CurveSpacing, CurveStation, Datum, DatumAxis, DatumFrame,
    DatumPlane, DatumPoint, EdgeGroup, Extrude, ExtrudeEnd, ExtrudeExtent, FaceAxisMate, FaceMate,
    FaceOnRound, FacePair, FaceTangent, FaceThickness, FeatureId, FeatureKind, FinePitch, Hole,
    HoleBottom, HoleDepth, HoleFit, HoleShape, HoleSizing, HoleStandard, HoleStep, HoleStyle,
    ImagePlane, ImageSize, ImageSpot, Import, LinearDirection, LinearSpacing, Mate, MatePair,
    MeasuredItem, Measurement, MetricSize, Mirror, Move, Of, OffsetFace, Pattern, PatternKind,
    Picture, Pipe, PixelSize, PlaneRotation, PlaneThrough, PointBy, PointMate, PointTarget,
    PointsPattern, Primitive, PrimitiveAnchor, PrimitiveShape, PrincipalPlane, Reading,
    ReferenceImage, RegionChoice, Remove, Revolve, RevolveAxis, RevolveExtent, Scale, Shell,
    ShellSide, SketchAttachment, SketchFeature, SolidFeature, SolidStart, Split, SplitAlong,
    SplitCarry, SplitFace, StepText, Sweep, SweepPath, TappedThread, Thread, ThreadFamily,
    ThreadForm, ThreadHand, ThreadLength, ThreadSide, TurnCentre, Wall, sketch_regions,
};
use caditor_expression::Expression;
use caditor_geometry::{Plane, Point2};
use caditor_kernel::{
    LinearExtent, PipeShape, Profile, ProfileCurve, RegionReference, Selection, Solid,
    SweepOrientation, WallSide, extrude, interruptible,
};
use caditor_sketch::EntityId;
use caditor_step::{StepBody, write_step};
use libfuzzer_sys::arbitrary::{Result, Unstructured};

use super::references::Scene;
use crate::{WORK_BUDGET, interrupt_after, plane, sketch::sketch};

const MOST_REGIONS: usize = 4;
const MOST_STEPS: u8 = 3;
const MOST_SKIPPED: u8 = 3;
const IMPORT_SIDE: f64 = 10.0;
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
const MOST_PICTURE_SIDE: u32 = 64;
const IMPORT_FEATURE: u64 = 1;
const IMPORT_SOURCES: [&str; 3] = ["block.step", "bracket.stp", "Ünïcode part.step"];
const WALL_SIDES: [WallSide; 3] = [WallSide::Inside, WallSide::Outside, WallSide::Centred];
const OPERATIONS: [CombineOperation; 3] = [
    CombineOperation::Join,
    CombineOperation::Cut,
    CombineOperation::Intersect,
];
const SIDES: [ThreadSide; 2] = [ThreadSide::Internal, ThreadSide::External];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Sketch,
    Solid,
    Blend,
    Shell,
    OffsetFace,
    SplitFace,
    Primitive,
    Pipe,
    Combine,
    Move,
    Mate,
    Mirror,
    Split,
    Scale,
    Hole,
    Pattern,
    Datum,
    Import,
    Remove,
    Thread,
    Measurement,
    ReferenceImage,
}

impl Family {
    pub const ALL: [Self; 22] = [
        Self::Sketch,
        Self::Solid,
        Self::Blend,
        Self::Shell,
        Self::OffsetFace,
        Self::SplitFace,
        Self::Primitive,
        Self::Pipe,
        Self::Combine,
        Self::Move,
        Self::Mate,
        Self::Mirror,
        Self::Split,
        Self::Scale,
        Self::Hole,
        Self::Pattern,
        Self::Datum,
        Self::Import,
        Self::Remove,
        Self::Thread,
        Self::Measurement,
        Self::ReferenceImage,
    ];

    pub fn of(kind: &FeatureKind) -> Self {
        match kind {
            FeatureKind::Sketch(_) => Self::Sketch,
            FeatureKind::Solid(_) => Self::Solid,
            FeatureKind::Blend(_) => Self::Blend,
            FeatureKind::Shell(_) => Self::Shell,
            FeatureKind::OffsetFace(_) => Self::OffsetFace,
            FeatureKind::SplitFace(_) => Self::SplitFace,
            FeatureKind::Primitive(_) => Self::Primitive,
            FeatureKind::Pipe(_) => Self::Pipe,
            FeatureKind::Combine(_) => Self::Combine,
            FeatureKind::Move(_) => Self::Move,
            FeatureKind::Mate(_) => Self::Mate,
            FeatureKind::Mirror(_) => Self::Mirror,
            FeatureKind::Split(_) => Self::Split,
            FeatureKind::Scale(_) => Self::Scale,
            FeatureKind::Hole(_) => Self::Hole,
            FeatureKind::Pattern(_) => Self::Pattern,
            FeatureKind::Datum(_) => Self::Datum,
            FeatureKind::Import(_) => Self::Import,
            FeatureKind::Remove(_) => Self::Remove,
            FeatureKind::Thread(_) => Self::Thread,
            FeatureKind::Measurement(_) => Self::Measurement,
            FeatureKind::ReferenceImage(_) => Self::ReferenceImage,
        }
    }
}

pub fn feature_kind(
    input: &mut Unstructured,
    scene: &Scene,
    family: Family,
) -> Result<Option<FeatureKind>> {
    Ok(Some(match family {
        Family::Sketch => sketch_kind(input, scene)?,
        Family::Solid => return solid(input, scene),
        Family::Blend => FeatureKind::Blend(blend(input, scene)?),
        Family::Shell => {
            let body = scene.body(input)?;
            FeatureKind::Shell(Shell {
                body,
                open: scene.faces(input, body)?,
                thickness: scene.distance(input)?,
                side: *input.choose(&ShellSide::ALL)?,
                faces: (0..input.int_in_range(0u8..=2)?)
                    .map(|_| {
                        Ok(FaceThickness {
                            face: scene.face(input, body)?,
                            thickness: scene.distance(input)?,
                        })
                    })
                    .collect::<Result<_>>()?,
            })
        }
        Family::OffsetFace => {
            let body = scene.body(input)?;
            FeatureKind::OffsetFace(OffsetFace {
                body,
                faces: scene.faces(input, body)?,
                distance: scene.distance(input)?,
                tangent: input.arbitrary()?,
            })
        }
        Family::SplitFace => {
            let body = scene.body(input)?;
            FeatureKind::SplitFace(SplitFace {
                body,
                faces: scene.faces(input, body)?,
                along: split_along(input, scene)?,
                carry: match input.int_in_range(0u8..=2)? {
                    0 => SplitCarry::Square,
                    1 => SplitCarry::Along(Box::new(scene.axis(input)?)),
                    _ => SplitCarry::Wrapped,
                },
            })
        }
        Family::Primitive => FeatureKind::Primitive(primitive(input, scene)?),
        Family::Pipe => FeatureKind::Pipe(Pipe {
            path: sweep_path(input, scene)?,
            shape: *input.choose(&PipeShape::ALL)?,
            size: scene.distance(input)?,
            wall: if input.arbitrary()? {
                Some(scene.distance(input)?)
            } else {
                None
            },
            bend: scene.distance(input)?,
            operation: operation(input, scene)?,
        }),
        Family::Combine => FeatureKind::Combine(Combine {
            body: scene.body(input)?,
            tool: scene.body(input)?,
            more_tools: scene.bodies(input)?,
            keep_tool: input.arbitrary()?,
            operation: *input.choose(&OPERATIONS)?,
        }),
        Family::Move => FeatureKind::Move(movement(input, scene)?),
        Family::Mate => FeatureKind::Mate(Mate {
            body: scene.body(input)?,
            pair: mate_pair(input, scene)?,
            flipped: input.arbitrary()?,
        }),
        Family::Mirror => {
            let body = scene.body(input)?;
            FeatureKind::Mirror(Mirror {
                body,
                plane: scene.plane(input)?,
                keep_original: input.arbitrary()?,
                mirrored: scene.bodies(input)?,
                faces: scene.faces(input, body)?,
            })
        }
        Family::Split => FeatureKind::Split(Split {
            body: scene.body(input)?,
            along: split_along(input, scene)?,
            flipped: input.arbitrary()?,
        }),
        Family::Scale => FeatureKind::Scale(Scale {
            body: scene.body(input)?,
            factor: scene.factor(input)?,
            center: [
                scene.distance(input)?,
                scene.distance(input)?,
                scene.distance(input)?,
            ],
            frame: frame(input, scene)?,
        }),
        Family::Hole => FeatureKind::Hole(hole(input, scene)?),
        Family::Pattern => FeatureKind::from(pattern(input, scene)?),
        Family::Datum => FeatureKind::Datum(datum(input, scene)?),
        Family::Import => return import(input, scene),
        Family::Remove => FeatureKind::Remove(Remove {
            body: scene.body(input)?,
        }),
        Family::Thread => FeatureKind::Thread(thread(input, scene)?),
        Family::Measurement => FeatureKind::from(Measurement {
            reading: reading(input, scene)?,
            parameter: if input.ratio(1u8, 4)? {
                scene.parameter(input)?
            } else {
                None
            },
        }),
        Family::ReferenceImage => return reference_image(input, scene),
    }))
}

fn reference_image(input: &mut Unstructured, scene: &Scene) -> Result<Option<FeatureKind>> {
    let mut bytes = PNG_SIGNATURE.to_vec();
    let length = usize::from(input.int_in_range(0u8..=32)?);
    bytes.extend(input.bytes(length)?);
    let size = PixelSize {
        width: input.int_in_range(1..=MOST_PICTURE_SIDE)?,
        height: input.int_in_range(1..=MOST_PICTURE_SIDE)?,
    };
    let Ok(picture) = Picture::new(bytes, size) else {
        return Ok(None);
    };
    let plane = if input.arbitrary()? {
        ImagePlane::Sketch(scene.sketch(input)?)
    } else {
        ImagePlane::Reference(scene.plane(input)?)
    };
    let spot = |input: &mut Unstructured| -> Result<ImageSpot> {
        Ok(ImageSpot::new(
            f64::from(input.int_in_range(0u8..=8)?) / 8.0,
            f64::from(input.int_in_range(0u8..=8)?) / 8.0,
        ))
    };
    let size = if input.arbitrary()? {
        ImageSize::Width(scene.distance(input)?)
    } else {
        ImageSize::Calibrated(Calibration {
            from: spot(input)?,
            to: spot(input)?,
            distance: scene.distance(input)?,
        })
    };
    Ok(Some(FeatureKind::ReferenceImage(Box::new(
        ReferenceImage {
            picture: Arc::new(picture),
            plane,
            at: [scene.distance(input)?, scene.distance(input)?],
            turn: scene.turn(input)?,
            flipped: input.arbitrary()?,
            size,
            opacity: input.arbitrary()?,
            in_front: input.arbitrary()?,
            locked: input.arbitrary()?,
        },
    ))))
}

fn sketch_kind(input: &mut Unstructured, scene: &Scene) -> Result<FeatureKind> {
    let on = plane(input)?;
    let drawn = sketch(input, on)?;
    let attachment = match input.int_in_range(0u8..=5)? {
        0 => Some(SketchAttachment::Face(Box::new(
            scene.face_attachment(input)?,
        ))),
        1 => Some(SketchAttachment::Datum(scene.datum(input)?)),
        2 => Some(SketchAttachment::Frame {
            frame: scene.datum(input)?,
            plane: *input.choose(&PrincipalPlane::ALL)?,
        }),
        _ => None,
    };
    let mut feature = SketchFeature::from(drawn);
    feature.attachment = attachment;
    Ok(FeatureKind::Sketch(feature))
}

pub fn operation(input: &mut Unstructured, scene: &Scene) -> Result<BodyOperation> {
    Ok(match input.int_in_range(0u8..=3)? {
        0 => BodyOperation::NewBody,
        1 => BodyOperation::Add(scene.body(input)?),
        2 => BodyOperation::Remove(scene.body(input)?),
        _ => BodyOperation::Intersect(scene.body(input)?),
    })
}

fn optional_distance(input: &mut Unstructured, scene: &Scene) -> Result<Option<Box<Expression>>> {
    Ok(if input.arbitrary::<bool>()? {
        Some(Box::new(scene.distance(input)?))
    } else {
        None
    })
}

fn extrude_end(input: &mut Unstructured, scene: &Scene) -> Result<ExtrudeEnd> {
    Ok(match input.int_in_range(0u8..=4)? {
        0 => ExtrudeEnd::ThroughAll,
        1 => ExtrudeEnd::UpToNext {
            offset: optional_distance(input, scene)?,
        },
        2 => ExtrudeEnd::UpToFace {
            target: Box::new(scene.plane(input)?),
            offset: optional_distance(input, scene)?,
        },
        3 => ExtrudeEnd::UpToSurface {
            face: Box::new(scene.face_attachment(input)?),
        },
        _ => ExtrudeEnd::Distance(scene.distance(input)?),
    })
}

fn regions(
    input: &mut Unstructured,
    scene: &Scene,
    sketch: Option<FeatureId>,
) -> Result<RegionChoice> {
    if !input.ratio(1u8, 4)? {
        return Ok(RegionChoice::All);
    }
    let drawn = sketch
        .and_then(|sketch| scene.document.feature(sketch))
        .and_then(|feature| feature.kind.sketch());
    let found = drawn
        .map(|drawn| interruptible(interrupt_after(WORK_BUDGET), || sketch_regions(drawn)))
        .and_then(std::result::Result::ok)
        .unwrap_or_default();
    let mut chosen = Vec::new();
    for region in found.iter().take(MOST_REGIONS) {
        if input.arbitrary::<bool>()? {
            let anchor = if input.arbitrary::<bool>()? {
                Some(crate::point(input)?)
            } else {
                None
            };
            chosen.push(RegionReference::capture(region, anchor));
        }
    }
    Ok(RegionChoice::Chosen(chosen))
}

fn start(input: &mut Unstructured, scene: &Scene) -> Result<Option<SolidStart>> {
    Ok(match input.int_in_range(0u8..=5)? {
        0 => Some(SolidStart::Distance(scene.distance(input)?)),
        1 => Some(SolidStart::Plane(scene.plane(input)?)),
        _ => None,
    })
}

fn wall(input: &mut Unstructured, scene: &Scene) -> Result<Option<Box<Wall>>> {
    Ok(if input.ratio(1u8, 6)? {
        Some(Box::new(Wall {
            thickness: scene.distance(input)?,
            side: *input.choose(&WALL_SIDES)?,
        }))
    } else {
        None
    })
}

fn sweep_path(input: &mut Unstructured, scene: &Scene) -> Result<SweepPath> {
    Ok(match input.int_in_range(0u8..=2)? {
        0 => SweepPath::Unchosen,
        1 => match scene.existing_sketch(input)? {
            Some(path) => {
                let lines = scene.sketch_lines(path);
                let mut curves = Vec::new();
                for _ in 0..input.int_in_range(1u8..=3)? {
                    curves.push(if lines.is_empty() || input.ratio(1u8, 8)? {
                        EntityId::from_raw(input.int_in_range(0u64..=32)?)
                    } else {
                        *input.choose(&lines)?
                    });
                }
                SweepPath::Curves {
                    sketch: path,
                    curves,
                }
            }
            None => SweepPath::Unchosen,
        },
        _ => {
            let body = scene.body(input)?;
            SweepPath::Edges {
                body,
                edges: scene.edges(input, body)?,
            }
        }
    })
}

fn solid(input: &mut Unstructured, scene: &Scene) -> Result<Option<FeatureKind>> {
    let Some(sketch) = scene.existing_sketch(input)? else {
        return Ok(None);
    };
    let regions = regions(input, scene, Some(sketch))?;
    let operation = operation(input, scene)?;
    let start = start(input, scene)?;
    let other_bodies = if input.ratio(1u8, 6)? {
        scene.bodies(input)?
    } else {
        Vec::new()
    };
    let wall = wall(input, scene)?;
    if input.ratio(1u8, 4)? {
        return Ok(Some(FeatureKind::Solid(SolidFeature::Sweep(Sweep {
            sketch,
            regions,
            path: sweep_path(input, scene)?,
            orientation: *input.choose(&[SweepOrientation::Square, SweepOrientation::Parallel])?,
            operation,
            other_bodies,
        }))));
    }
    let solid = if input.arbitrary::<bool>()? {
        let extent = match input.int_in_range(0u8..=2)? {
            0 => ExtrudeExtent::OneSide {
                end: extrude_end(input, scene)?,
                reversed: input.arbitrary()?,
            },
            1 => ExtrudeExtent::Symmetric {
                distance: scene.distance(input)?,
            },
            _ => ExtrudeExtent::TwoSides {
                forward: extrude_end(input, scene)?,
                backward: extrude_end(input, scene)?,
            },
        };
        SolidFeature::Extrude(Extrude {
            sketch,
            regions,
            extent,
            operation,
            start,
            other_bodies,
            taper: if input.ratio(1u8, 6)? {
                Some(Box::new(scene.turn(input)?))
            } else {
                None
            },
            wall,
            direction: if input.ratio(1u8, 6)? {
                Some(Box::new(scene.axis(input)?))
            } else {
                None
            },
        })
    } else {
        let lines = scene.sketch_lines(sketch);
        let axis = if !lines.is_empty() && input.arbitrary::<bool>()? {
            RevolveAxis::Sketch(*input.choose(&lines)?)
        } else if input.arbitrary::<bool>()? {
            RevolveAxis::Sketch(*input.choose(&EntityId::REFERENCES)?)
        } else {
            RevolveAxis::Model(scene.axis(input)?)
        };
        let extent = match input.int_in_range(0u8..=4)? {
            0 => RevolveExtent::Full,
            1 => RevolveExtent::OneSide {
                angle: scene.turn(input)?,
                reversed: input.arbitrary()?,
            },
            2 => RevolveExtent::Symmetric {
                angle: scene.turn(input)?,
            },
            3 => RevolveExtent::TwoSides {
                forward: scene.turn(input)?,
                backward: scene.turn(input)?,
            },
            _ => RevolveExtent::UpTo {
                target: Box::new(scene.plane(input)?),
                reversed: input.arbitrary()?,
            },
        };
        SolidFeature::Revolve(Revolve {
            sketch,
            regions,
            axis,
            extent,
            operation,
            start,
            other_bodies,
            side: match input.int_in_range(0u8..=2)? {
                0 => None,
                1 => Some(AxisSide::Left),
                _ => Some(AxisSide::Right),
            },
            wall,
        })
    };
    Ok(Some(FeatureKind::Solid(solid)))
}

fn blend(input: &mut Unstructured, scene: &Scene) -> Result<Blend> {
    let body = scene.body(input)?;
    let kind = if input.arbitrary::<bool>()? {
        BlendKind::Fillet
    } else {
        BlendKind::Chamfer
    };
    let mut groups = Vec::new();
    for _ in 0..input.int_in_range(0u8..=2)? {
        groups.push(if input.arbitrary::<bool>()? {
            EdgeGroup::Face(scene.face(input, body)?)
        } else {
            EdgeGroup::Body
        });
    }
    Ok(Blend {
        kind,
        body,
        edges: scene.edges(input, body)?,
        groups,
        size: scene.distance(input)?,
        form: match input.int_in_range(0u8..=2)? {
            0 => ChamferForm::Equal,
            1 => ChamferForm::TwoDistances {
                second: scene.distance(input)?,
            },
            _ => ChamferForm::DistanceAngle {
                angle: scene.turn(input)?,
            },
        },
        flipped: input.arbitrary()?,
    })
}

fn split_along(input: &mut Unstructured, scene: &Scene) -> Result<SplitAlong> {
    Ok(match input.int_in_range(0u8..=3)? {
        0 => SplitAlong::Plane(scene.plane(input)?),
        1 => SplitAlong::Body(scene.body(input)?),
        2 => SplitAlong::Sketch(scene.sketch(input)?),
        _ => SplitAlong::Surface(scene.face_attachment(input)?),
    })
}

fn primitive(input: &mut Unstructured, scene: &Scene) -> Result<Primitive> {
    let shape = match input.int_in_range(0u8..=6)? {
        0 => PrimitiveShape::Box {
            length: scene.distance(input)?,
            width: scene.distance(input)?,
            height: scene.distance(input)?,
        },
        1 => PrimitiveShape::Cylinder {
            diameter: scene.distance(input)?,
            height: scene.distance(input)?,
        },
        2 => PrimitiveShape::Sphere {
            diameter: scene.distance(input)?,
        },
        3 => PrimitiveShape::Torus {
            diameter: scene.distance(input)?,
            tube: scene.distance(input)?,
        },
        4 => PrimitiveShape::Cone {
            bottom: scene.distance(input)?,
            top: scene.distance(input)?,
            height: scene.distance(input)?,
        },
        5 => PrimitiveShape::Wedge {
            length: scene.distance(input)?,
            width: scene.distance(input)?,
            height: scene.distance(input)?,
            top: scene.distance(input)?,
        },
        _ => PrimitiveShape::Prism {
            sides: scene.count(input)?,
            diameter: scene.distance(input)?,
            height: scene.distance(input)?,
        },
    };
    Ok(Primitive {
        shape,
        plane: scene.plane(input)?,
        at: [scene.distance(input)?, scene.distance(input)?],
        anchor: *input.choose(&PrimitiveAnchor::ALL)?,
        reversed: input.arbitrary()?,
        operation: operation(input, scene)?,
    })
}

fn frame(input: &mut Unstructured, scene: &Scene) -> Result<Option<FeatureId>> {
    Ok(if input.ratio(1u8, 4)? {
        Some(scene.datum(input)?)
    } else {
        None
    })
}

fn movement(input: &mut Unstructured, scene: &Scene) -> Result<Move> {
    Ok(Move {
        body: scene.body(input)?,
        offset: [
            scene.distance(input)?,
            scene.distance(input)?,
            scene.distance(input)?,
        ],
        turn: [scene.turn(input)?, scene.turn(input)?, scene.turn(input)?],
        copy: input.arbitrary()?,
        about: match input.int_in_range(0u8..=2)? {
            0 => TurnCentre::Origin,
            1 => TurnCentre::Body,
            _ => TurnCentre::Axis(Box::new(AxisTurn {
                axis: scene.axis(input)?,
                angle: scene.turn(input)?,
            })),
        },
        frame: frame(input, scene)?,
    })
}

fn face_mate(input: &mut Unstructured, scene: &Scene) -> Result<FaceMate> {
    let body = scene.body(input)?;
    Ok(FaceMate {
        face: scene.face(input, body)?,
        target: scene.plane(input)?,
        distance: scene.distance(input)?,
    })
}

fn axis_mate(input: &mut Unstructured, scene: &Scene) -> Result<AxisMate> {
    Ok(AxisMate {
        axis: scene.axis(input)?,
        target: scene.axis(input)?,
    })
}

fn face_pair(input: &mut Unstructured, scene: &Scene) -> Result<FacePair> {
    let body = scene.body(input)?;
    Ok(FacePair {
        face: scene.face(input, body)?,
        target: scene.plane(input)?,
    })
}

fn mate_pair(input: &mut Unstructured, scene: &Scene) -> Result<MatePair> {
    Ok(match input.int_in_range(0u8..=6)? {
        0 => MatePair::Faces(Box::new(face_mate(input, scene)?)),
        1 => MatePair::Axes(Box::new(axis_mate(input, scene)?)),
        2 => MatePair::FaceAxis(Box::new(FaceAxisMate {
            faces: face_mate(input, scene)?,
            axes: axis_mate(input, scene)?,
        })),
        3 => MatePair::Angle(Box::new(AngleMate {
            sides: if input.arbitrary::<bool>()? {
                AngleSides::Faces(face_pair(input, scene)?)
            } else {
                AngleSides::Axes(axis_mate(input, scene)?)
            },
            angle: scene.turn(input)?,
        })),
        4 => MatePair::Tangent(Box::new(face_pair(input, scene)?)),
        5 => MatePair::Point(Box::new(PointMate {
            point: scene.point(input)?,
            target: if input.arbitrary::<bool>()? {
                PointTarget::Point(scene.point(input)?)
            } else {
                PointTarget::Plane(scene.plane(input)?)
            },
        })),
        _ => {
            let body = scene.body(input)?;
            MatePair::FaceOnRound(Box::new(FaceOnRound {
                face: scene.face(input, body)?,
                round: scene.face_attachment(input)?,
            }))
        }
    })
}

fn hole_depth(input: &mut Unstructured, scene: &Scene) -> Result<HoleDepth> {
    Ok(match input.int_in_range(0u8..=3)? {
        0 => HoleDepth::Blind(scene.distance(input)?),
        1 => HoleDepth::ThroughAll,
        2 => HoleDepth::UpToNext {
            offset: optional_distance(input, scene)?,
        },
        _ => HoleDepth::UpToFace {
            target: Box::new(scene.plane(input)?),
            offset: optional_distance(input, scene)?,
        },
    })
}

fn hole_style(input: &mut Unstructured, scene: &Scene) -> Result<HoleStyle> {
    Ok(match input.int_in_range(0u8..=3)? {
        0 => HoleStyle::Plain,
        1 => HoleStyle::Counterbore {
            diameter: scene.distance(input)?,
            depth: scene.distance(input)?,
        },
        2 => HoleStyle::Countersink {
            diameter: scene.distance(input)?,
            angle: scene.turn(input)?,
        },
        _ => {
            let mut steps = Vec::new();
            for _ in 0..input.int_in_range(0..=MOST_STEPS)? {
                steps.push(HoleStep {
                    diameter: scene.distance(input)?,
                    depth: scene.distance(input)?,
                });
            }
            HoleStyle::Stepped(steps)
        }
    })
}

fn hole_standard(input: &mut Unstructured) -> Result<Option<HoleStandard>> {
    if input.arbitrary::<bool>()? {
        return Ok(None);
    }
    let fit = match input.int_in_range(0u8..=5)? {
        0 => HoleFit::Close,
        1 => HoleFit::Normal,
        2 => HoleFit::Loose,
        3 => HoleFit::Tapped,
        4 => HoleFit::TappedFine(*input.choose(&FinePitch::ALL)?),
        _ => HoleFit::HeatSetInsert,
    };
    Ok(Some(HoleStandard::offered(
        *input.choose(&MetricSize::ALL)?,
        fit,
    )))
}

fn hole(input: &mut Unstructured, scene: &Scene) -> Result<Hole> {
    let classes = ThreadFamily::MetricCoarse.classes(ThreadSide::Internal);
    Ok(Hole {
        sketch: scene.sketch(input)?,
        body: scene.body(input)?,
        diameter: scene.distance(input)?,
        depth: hole_depth(input, scene)?,
        style: hole_style(input, scene)?,
        reversed: input.arbitrary()?,
        shape: if input.arbitrary::<bool>()? {
            HoleShape::Round
        } else {
            HoleShape::Slot {
                length: scene.distance(input)?,
                angle: scene.turn(input)?,
            }
        },
        standard: hole_standard(input)?,
        sizing: match input.int_in_range(0u8..=2)? {
            0 => HoleSizing::Typed,
            1 => HoleSizing::Circles,
            _ => HoleSizing::CirclesAndHeads,
        },
        bottom: if input.arbitrary::<bool>()? {
            HoleBottom::Flat
        } else {
            HoleBottom::DrillPoint(scene.turn(input)?)
        },
        thread: TappedThread {
            class: if input.arbitrary::<bool>()? {
                Some(*input.choose(classes)?)
            } else {
                None
            },
            hand: *input.choose(&ThreadHand::ALL)?,
            depth: if input.arbitrary::<bool>()? {
                Some(scene.distance(input)?)
            } else {
                None
            },
            form: thread_form(input)?,
        },
    })
}

fn linear_direction(input: &mut Unstructured, scene: &Scene) -> Result<LinearDirection> {
    Ok(LinearDirection {
        axis: scene.axis(input)?,
        count: scene.count(input)?,
        spacing: scene.distance(input)?,
        measured: if input.arbitrary::<bool>()? {
            LinearSpacing::BetweenCopies
        } else {
            LinearSpacing::Total
        },
        reversed: input.arbitrary()?,
    })
}

fn pattern(input: &mut Unstructured, scene: &Scene) -> Result<Pattern> {
    let kind = match input.int_in_range(0u8..=3)? {
        0 => PatternKind::Linear {
            first: linear_direction(input, scene)?,
            second: if input.arbitrary::<bool>()? {
                Some(linear_direction(input, scene)?)
            } else {
                None
            },
        },
        1 => PatternKind::Circular(CircularPattern {
            axis: if input.ratio(1, 8)? {
                None
            } else {
                Some(scene.axis(input)?)
            },
            count: scene.count(input)?,
            angle: scene.turn(input)?,
            reversed: input.arbitrary()?,
        }),
        2 => PatternKind::Curve(CurvePattern {
            sketch: scene.sketch(input)?,
            count: scene.count(input)?,
            spacing: scene.distance(input)?,
            measured: if input.arbitrary::<bool>()? {
                CurveSpacing::Spread
            } else {
                CurveSpacing::Distance
            },
            orientation: if input.arbitrary::<bool>()? {
                CopyOrientation::Kept
            } else {
                CopyOrientation::Following
            },
            reversed: input.arbitrary()?,
        }),
        _ => PatternKind::Points(PointsPattern {
            sketch: scene.sketch(input)?,
            base: scene.point(input)?,
        }),
    };
    let mut skipped = BTreeSet::new();
    for _ in 0..input.int_in_range(0..=MOST_SKIPPED)? {
        skipped.insert([input.int_in_range(0u32..=3)?, input.int_in_range(0u32..=3)?]);
    }
    Ok(Pattern {
        body: scene.body(input)?,
        kind,
        skipped,
        repeated: if input.ratio(1u8, 4)? {
            scene.bodies(input)?
        } else {
            Vec::new()
        },
    })
}

fn curve_station(input: &mut Unstructured, scene: &Scene) -> Result<Box<CurveStation>> {
    let body = scene.body(input)?;
    Ok(Box::new(CurveStation {
        body,
        edge: Box::new(scene.edge(input, body)?),
        distance: scene.distance(input)?,
    }))
}

fn face_tangent(input: &mut Unstructured, scene: &Scene) -> Result<Box<FaceTangent>> {
    let body = scene.body(input)?;
    Ok(Box::new(FaceTangent {
        body,
        face: scene.face(input, body)?,
        toward: scene.point(input)?,
    }))
}

pub fn datum(input: &mut Unstructured, scene: &Scene) -> Result<Datum> {
    Ok(match input.int_in_range(0u8..=5)? {
        0 => Datum::Plane(DatumPlane {
            base: scene.plane(input)?,
            rotation: if input.ratio(1u8, 4)? {
                Some(PlaneRotation {
                    axis: scene.axis(input)?,
                    angle: scene.turn(input)?,
                })
            } else {
                None
            },
            offset: scene.distance(input)?,
        }),
        1 => Datum::PlaneThrough(match input.int_in_range(0u8..=7)? {
            0 => PlaneThrough::Points([
                scene.point(input)?,
                scene.point(input)?,
                scene.point(input)?,
            ]),
            1 => PlaneThrough::Midway(scene.plane(input)?, scene.plane(input)?),
            2 => PlaneThrough::AxisAndPoint(scene.axis(input)?, scene.point(input)?),
            3 => PlaneThrough::NormalTo(scene.axis(input)?, scene.point(input)?),
            4 => PlaneThrough::Tangent(face_tangent(input, scene)?),
            5 => PlaneThrough::SquareToCurve(curve_station(input, scene)?),
            6 => PlaneThrough::Lines(scene.axis(input)?, scene.axis(input)?),
            _ => PlaneThrough::TangentAt(face_tangent(input, scene)?),
        }),
        2 => Datum::Axis(match input.int_in_range(0u8..=4)? {
            0 => DatumAxis::Along(scene.axis(input)?),
            1 => DatumAxis::Intersection(scene.plane(input)?, scene.plane(input)?),
            2 => DatumAxis::Points(scene.point(input)?, scene.point(input)?),
            3 => DatumAxis::NormalTo(scene.plane(input)?, scene.point(input)?),
            _ => DatumAxis::SquareToFace(face_tangent(input, scene)?),
        }),
        3 => Datum::Point(DatumPoint {
            base: scene.point(input)?,
            offset: [
                scene.distance(input)?,
                scene.distance(input)?,
                scene.distance(input)?,
            ],
        }),
        4 => Datum::PointBy(match input.int_in_range(0u8..=5)? {
            0 => PointBy::LinesCross(scene.axis(input)?, scene.axis(input)?),
            1 => PointBy::AxisAndPlane(scene.axis(input)?, scene.plane(input)?),
            2 => PointBy::ThreePlanes([
                scene.plane(input)?,
                scene.plane(input)?,
                scene.plane(input)?,
            ]),
            3 => PointBy::Along(curve_station(input, scene)?),
            4 => {
                let body = scene.body(input)?;
                PointBy::EdgeMiddle {
                    body,
                    edge: Box::new(scene.edge(input, body)?),
                }
            }
            _ => {
                let body = scene.body(input)?;
                PointBy::FaceCentre {
                    body,
                    face: scene.face(input, body)?,
                }
            }
        }),
        _ => Datum::Frame(Box::new(DatumFrame {
            origin: scene.point(input)?,
            x_axis: scene.axis(input)?,
            plane: scene.plane(input)?,
            reverse_x: input.arbitrary()?,
            reverse_z: input.arbitrary()?,
        })),
    })
}

fn imported_block() -> Option<(Arc<Solid>, StepText)> {
    static BLOCK: OnceLock<Option<(Arc<Solid>, StepText)>> = OnceLock::new();
    BLOCK
        .get_or_init(|| {
            let corners = [
                Point2::new(0.0, 0.0),
                Point2::new(IMPORT_SIDE, 0.0),
                Point2::new(IMPORT_SIDE, IMPORT_SIDE),
                Point2::new(0.0, IMPORT_SIDE),
            ];
            let curves: Vec<ProfileCurve> = (0..corners.len())
                .filter_map(|index| {
                    let start = corners.get(index)?;
                    let end = corners.get((index + 1) % corners.len())?;
                    Some(ProfileCurve::line(index as u64 + 1, *start, *end))
                })
                .collect();
            let regions = Profile::new(&curves)
                .ok()?
                .select(&Selection::EvenDepth)
                .ok()?;
            let extent = LinearExtent::one_side(IMPORT_SIDE).ok()?;
            let solid = extrude(&Plane::XY, &regions, extent, IMPORT_FEATURE).ok()?;
            let body = StepBody {
                name: "Block",
                solid: &solid,
                colour: None,
                opacity: None,
                layer: None,
                threads: &[],
                faces: &[],
            };
            let step = write_step(&[body], "Block", SystemTime::UNIX_EPOCH).ok()?;
            Some((Arc::new(solid), StepText::new(&step)))
        })
        .clone()
}

fn import(input: &mut Unstructured, scene: &Scene) -> Result<Option<FeatureKind>> {
    let Some((solid, step)) = imported_block() else {
        return Ok(None);
    };
    let placement = BodyPlacement {
        offset: [
            scene.distance(input)?,
            scene.distance(input)?,
            scene.distance(input)?,
        ],
        turn: [scene.turn(input)?, scene.turn(input)?, scene.turn(input)?],
        frame: frame(input, scene)?,
        scale: scene.factor(input)?,
    };
    let source = *input.choose(&IMPORT_SOURCES)?;
    let mut imported = Import::shared(source, solid, step);
    if input.arbitrary::<bool>()? {
        imported = imported.from_file(std::path::PathBuf::from("/models").join(source));
    }
    if input.arbitrary::<bool>()? {
        imported = imported.placed(placement);
    }
    Ok(Some(FeatureKind::Import(imported)))
}

fn thread(input: &mut Unstructured, scene: &Scene) -> Result<Thread> {
    let body = scene.body(input)?;
    let family = *input.choose(&ThreadFamily::ALL)?;
    let sizes = family.sizes();
    let size = *input.choose(&sizes)?;
    let classes = family.classes(*input.choose(&SIDES)?);
    Ok(Thread {
        body,
        face: scene.face(input, body)?,
        size,
        class: *input.choose(classes)?,
        hand: *input.choose(&ThreadHand::ALL)?,
        length: if input.arbitrary::<bool>()? {
            ThreadLength::Full
        } else {
            ThreadLength::Depth(scene.distance(input)?)
        },
        reversed: input.arbitrary()?,
        form: thread_form(input)?,
    })
}

fn thread_form(input: &mut Unstructured) -> Result<ThreadForm> {
    Ok(if input.arbitrary::<bool>()? {
        ThreadForm::Cosmetic
    } else {
        ThreadForm::Modelled
    })
}

fn measured_item(input: &mut Unstructured, scene: &Scene) -> Result<MeasuredItem> {
    Ok(match input.int_in_range(0u8..=6)? {
        0 => MeasuredItem::Point(scene.point(input)?),
        1 => MeasuredItem::Axis(scene.axis(input)?),
        2 => MeasuredItem::Plane(scene.plane(input)?),
        3 => {
            let body = scene.body(input)?;
            MeasuredItem::Edge {
                body,
                edge: Box::new(scene.edge(input, body)?),
            }
        }
        4 => {
            let body = scene.body(input)?;
            MeasuredItem::Face {
                body,
                face: scene.face(input, body)?,
            }
        }
        5 => {
            let sketch = scene.sketch(input)?;
            MeasuredItem::Sketch {
                sketch,
                entity: scene.sketch_entity(input, sketch)?,
            }
        }
        _ => MeasuredItem::Body(scene.body(input)?),
    })
}

fn reading(input: &mut Unstructured, scene: &Scene) -> Result<Reading> {
    Ok(match input.int_in_range(0u8..=3)? {
        0 => Reading::Between {
            quantity: *input.choose(&Between::ALL)?,
            first: measured_item(input, scene)?,
            second: measured_item(input, scene)?,
        },
        1 => Reading::Along {
            first: measured_item(input, scene)?,
            second: measured_item(input, scene)?,
            axis: measured_item(input, scene)?,
        },
        2 => Reading::Of {
            quantity: *input.choose(&Of::ALL)?,
            item: measured_item(input, scene)?,
        },
        _ => Reading::Position {
            item: measured_item(input, scene)?,
            axis: measured_item(input, scene)?,
        },
    })
}
