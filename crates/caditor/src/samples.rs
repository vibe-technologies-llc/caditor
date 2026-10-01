use anyhow::{Context, Result};
use caditor_document::{
    BodyOperation, Document, Extrude, ExtrudeExtent, FeatureId, FeatureKind, RegionChoice, Revolve,
    RevolveAxis, RevolveExtent, SolidFeature, TransactionBuilder,
};
use caditor_geometry::{Plane, Point2};
use caditor_sketch::{Constraint, Entity, EntityId, Sketch};

use crate::variants::all_variants;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Sample {
    Plate,
    Spool,
    Bracket,
}

all_variants!(Sample: Plate, Spool, Bracket);

impl Sample {
    pub fn title(self) -> &'static str {
        match self {
            Self::Plate => "Mounting plate",
            Self::Spool => "Flanged spool",
            Self::Bracket => "Angle bracket",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Plate => {
                "A plate with two holes, extruded from one fully constrained sketch. Change its \
                 length, width or hole in Parameters."
            }
            Self::Spool => {
                "A spool revolved from a profile about the sketch's vertical axis. Its diameters \
                 and heights are parameters."
            }
            Self::Bracket => {
                "An L-shaped bracket extruded both ways from its profile, with a hole cut by a \
                 second extrusion that removes material."
            }
        }
    }

    pub fn document(self) -> Result<Document> {
        let mut document = Document::default();
        let mut transaction = document.transaction(format!("Open the {} sample", self.title()));
        match self {
            Self::Plate => plate(&mut transaction)?,
            Self::Spool => spool(&mut transaction)?,
            Self::Bracket => bracket(&mut transaction)?,
        }
        document
            .apply(transaction.finish())
            .with_context(|| format!("the {} sample could not be built", self.title()))?;
        Ok(document)
    }
}

struct Builder<'t, 'd> {
    transaction: &'t mut TransactionBuilder<'d>,
    sketch: Sketch,
}

impl<'t, 'd> Builder<'t, 'd> {
    fn new(transaction: &'t mut TransactionBuilder<'d>, plane: Plane) -> Self {
        Self {
            transaction,
            sketch: Sketch::new(plane),
        }
    }

    fn outline(&mut self, corners: &[Point2]) -> Result<(Vec<EntityId>, Vec<EntityId>)> {
        let points: Vec<EntityId> = corners
            .iter()
            .map(|corner| self.sketch.add_point(*corner))
            .collect();
        let mut lines = Vec::with_capacity(points.len());
        for (index, start) in points.iter().enumerate() {
            let end = points
                .get((index + 1) % points.len())
                .context("an outline needs corners")?;
            let id = EntityId::from_raw(self.sketch.next_id());
            self.sketch.insert_entity(
                id,
                Entity::Line {
                    start: *start,
                    end: *end,
                },
            )?;
            lines.push(id);
        }
        Ok((points, lines))
    }

    fn level(&mut self, lines: &[EntityId]) -> Result<()> {
        for (index, line) in lines.iter().enumerate() {
            let constraint = if index % 2 == 0 {
                Constraint::Horizontal(*line)
            } else {
                Constraint::Vertical(*line)
            };
            self.sketch.add_constraint(constraint)?;
        }
        Ok(())
    }

    fn distance(&mut self, from: EntityId, to: EntityId, value: &str) -> Result<()> {
        let value = self.transaction.parse(value)?;
        self.sketch
            .add_constraint(Constraint::Distance { from, to, value })?;
        Ok(())
    }

    fn radius(&mut self, entity: EntityId, value: &str) -> Result<()> {
        let value = self.transaction.parse(value)?;
        self.sketch
            .add_constraint(Constraint::Radius { entity, value })?;
        Ok(())
    }

    fn center(&self, circle: EntityId) -> Result<EntityId> {
        match self.sketch.entity(circle) {
            Some(Entity::Circle { center, .. }) => Ok(*center),
            _ => anyhow::bail!("entity {circle} is not a circle"),
        }
    }

    fn add(self, name: &str) -> FeatureId {
        self.transaction
            .add_feature(name, FeatureKind::from(self.sketch))
    }
}

fn parameters(transaction: &mut TransactionBuilder<'_>, values: &[(&str, &str)]) -> Result<()> {
    for (name, text) in values {
        let expression = transaction.parse(text)?;
        transaction.add_parameter(*name, expression);
    }
    Ok(())
}

fn extrude(sketch: FeatureId, extent: ExtrudeExtent) -> FeatureKind {
    FeatureKind::Solid(SolidFeature::Extrude(Extrude {
        sketch,
        regions: RegionChoice::All,
        extent,
        operation: BodyOperation::NewBody,
    }))
}

fn plate(transaction: &mut TransactionBuilder<'_>) -> Result<()> {
    parameters(
        transaction,
        &[
            ("length", "80 mm"),
            ("width", "50 mm"),
            ("thickness", "6 mm"),
            ("hole", "8 mm"),
            ("margin", "10 mm"),
        ],
    )?;
    let mut builder = Builder::new(transaction, Plane::XY);
    let (corners, sides) = builder.outline(&[
        Point2::new(0.0, 0.0),
        Point2::new(80.0, 0.0),
        Point2::new(80.0, 50.0),
        Point2::new(0.0, 50.0),
    ])?;
    let (&[first, second, _, fourth], &[bottom, right, top, left]) =
        (corners.as_slice(), sides.as_slice())
    else {
        anyhow::bail!("the plate outline has four sides");
    };
    builder.level(&sides)?;
    builder
        .sketch
        .add_constraint(Constraint::Coincident(first, EntityId::ORIGIN))?;
    builder.distance(first, second, "length")?;
    builder.distance(first, fourth, "width")?;
    for (at, (across, along)) in [
        (Point2::new(10.0, 10.0), (left, bottom)),
        (Point2::new(70.0, 40.0), (right, top)),
    ] {
        let hole = builder.sketch.add_circle(at, 4.0);
        let center = builder.center(hole)?;
        builder.distance(center, across, "margin")?;
        builder.distance(center, along, "margin")?;
        builder.radius(hole, "hole / 2")?;
    }
    let sketch = builder.add("Plate sketch");
    let distance = transaction.parse("thickness")?;
    transaction.add_feature(
        "Plate",
        extrude(sketch, ExtrudeExtent::one_side(distance, false)),
    );
    Ok(())
}

fn spool(transaction: &mut TransactionBuilder<'_>) -> Result<()> {
    parameters(
        transaction,
        &[
            ("bore", "12 mm"),
            ("core", "36 mm"),
            ("flange", "60 mm"),
            ("height", "50 mm"),
            ("flange_thickness", "5 mm"),
        ],
    )?;
    let mut builder = Builder::new(transaction, Plane::XZ);
    let (points, sides) = builder.outline(&[
        Point2::new(6.0, 0.0),
        Point2::new(30.0, 0.0),
        Point2::new(30.0, 5.0),
        Point2::new(18.0, 5.0),
        Point2::new(18.0, 45.0),
        Point2::new(30.0, 45.0),
        Point2::new(30.0, 50.0),
        Point2::new(6.0, 50.0),
    ])?;
    let &[
        inner,
        flange_bottom,
        lower_step,
        core_bottom,
        core_top,
        upper_step,
        flange_top,
        _,
    ] = points.as_slice()
    else {
        anyhow::bail!("the spool profile has eight corners");
    };
    builder.level(&sides)?;
    builder
        .sketch
        .add_constraint(Constraint::Coincident(inner, EntityId::HORIZONTAL_AXIS))?;
    for (point, value) in [
        (inner, "bore / 2"),
        (flange_bottom, "flange / 2"),
        (core_bottom, "core / 2"),
        (upper_step, "flange / 2"),
    ] {
        builder.distance(point, EntityId::VERTICAL_AXIS, value)?;
    }
    builder.distance(flange_bottom, lower_step, "flange_thickness")?;
    builder.distance(core_bottom, core_top, "height - 2 * flange_thickness")?;
    builder.distance(upper_step, flange_top, "flange_thickness")?;
    let sketch = builder.add("Spool profile");
    transaction.add_feature(
        "Spool",
        FeatureKind::Solid(SolidFeature::Revolve(Revolve {
            sketch,
            regions: RegionChoice::All,
            axis: RevolveAxis::Sketch(EntityId::VERTICAL_AXIS),
            extent: RevolveExtent::Full,
            operation: BodyOperation::NewBody,
        })),
    );
    Ok(())
}

fn bracket(transaction: &mut TransactionBuilder<'_>) -> Result<()> {
    parameters(
        transaction,
        &[
            ("leg", "50 mm"),
            ("width", "40 mm"),
            ("thickness", "5 mm"),
            ("hole", "8 mm"),
        ],
    )?;
    let mut builder = Builder::new(transaction, Plane::XZ);
    let (points, sides) = builder.outline(&[
        Point2::new(0.0, 0.0),
        Point2::new(50.0, 0.0),
        Point2::new(50.0, 5.0),
        Point2::new(5.0, 5.0),
        Point2::new(5.0, 50.0),
        Point2::new(0.0, 50.0),
    ])?;
    let &[corner, toe, toe_top, _, top_inner, top_outer] = points.as_slice() else {
        anyhow::bail!("the bracket profile has six corners");
    };
    builder.level(&sides)?;
    builder
        .sketch
        .add_constraint(Constraint::Coincident(corner, EntityId::ORIGIN))?;
    builder.distance(corner, toe, "leg")?;
    builder.distance(toe, toe_top, "thickness")?;
    builder.distance(top_outer, corner, "leg")?;
    builder.distance(top_inner, top_outer, "thickness")?;
    let profile = builder.add("Bracket profile");
    let width = transaction.parse("width")?;
    let body = transaction.add_feature(
        "Bracket",
        extrude(profile, ExtrudeExtent::Symmetric { distance: width }),
    );

    let mut builder = Builder::new(transaction, Plane::XY);
    let hole = builder.sketch.add_circle(Point2::new(30.0, 0.0), 4.0);
    let center = builder.center(hole)?;
    builder
        .sketch
        .add_constraint(Constraint::Coincident(center, EntityId::HORIZONTAL_AXIS))?;
    builder.distance(center, EntityId::VERTICAL_AXIS, "leg - 20 mm")?;
    builder.radius(hole, "hole / 2")?;
    let hole_sketch = builder.add("Hole sketch");
    let depth = transaction.parse("2 * thickness + 2 mm")?;
    transaction.add_feature(
        "Hole",
        FeatureKind::Solid(SolidFeature::Extrude(Extrude {
            sketch: hole_sketch,
            regions: RegionChoice::All,
            extent: ExtrudeExtent::Symmetric { distance: depth },
            operation: BodyOperation::Remove(body),
        })),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use caditor_document::{CancelToken, FeatureState, ModelEvaluator, Recompute};

    use super::*;

    fn volume(sample: Sample, body: &str) -> f64 {
        let document = sample.document().unwrap();
        let evaluation = Recompute::default().run(
            &document,
            &ModelEvaluator,
            &CancelToken::never(),
            &|_, _| {},
        );
        for feature in document.features() {
            let status = evaluation.feature(feature.id()).unwrap();
            assert!(
                matches!(status.state, FeatureState::UpToDate),
                "{} in {}: {:?}",
                feature.name,
                sample.title(),
                status.state
            );
            if let Some(sketch) = status.result.as_ref().and_then(|result| result.sketch()) {
                assert_eq!(
                    sketch.solution.degrees_of_freedom(),
                    0,
                    "{} in {}",
                    feature.name,
                    sample.title()
                );
            }
        }
        let body = document
            .features()
            .find(|feature| feature.name == body)
            .unwrap()
            .id();
        let solid = evaluation.body(body).unwrap();
        let tolerance = solid.default_tolerance();
        solid
            .tessellate(&tolerance)
            .unwrap()
            .mass_properties()
            .volume
    }

    #[test]
    fn every_sample_recomputes_fully_constrained_into_the_shape_it_describes() {
        let hole = PI * 4.0 * 4.0;
        let plate = 80.0 * 50.0 * 6.0 - 2.0 * hole * 6.0;
        assert!((volume(Sample::Plate, "Plate") - plate).abs() / plate < 1e-3);
        let spool = PI * (30.0 * 30.0 - 6.0 * 6.0) * 10.0 + PI * (18.0 * 18.0 - 6.0 * 6.0) * 40.0;
        assert!((volume(Sample::Spool, "Spool") - spool).abs() / spool < 1e-2);
        let bracket = (50.0 * 5.0 + 45.0 * 5.0) * 40.0 - hole * 5.0;
        assert!((volume(Sample::Bracket, "Bracket") - bracket).abs() / bracket < 1e-3);
    }
}
