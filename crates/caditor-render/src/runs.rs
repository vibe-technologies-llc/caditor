use std::{fmt, sync::Arc};

use caditor_geometry::{Point3, RigidTransform};

use crate::scene::{Color, Layer, Line, PickId, StrokeKind};

pub trait PolylineSource: Send + Sync {
    fn polyline(&self, index: usize) -> &[u32];
    fn point(&self, index: u32) -> Option<Point3>;
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RunPolyline {
    pub polyline: u32,
    pub color: Color,
    pub width: f32,
    pub pick: Option<PickId>,
    pub stroke: StrokeKind,
}

#[derive(Clone)]
pub struct PolylineRun {
    pub source: Arc<dyn PolylineSource>,
    pub placement: Option<RigidTransform>,
    pub layer: Layer,
    pub polylines: Vec<RunPolyline>,
}

impl PolylineRun {
    pub fn lines(&self) -> impl Iterator<Item = Line> + '_ {
        self.polylines
            .iter()
            .flat_map(move |drawn| self.polyline_lines(drawn))
    }

    pub fn segment_count(&self) -> usize {
        self.polylines
            .iter()
            .map(|drawn| self.indices(drawn).len().saturating_sub(1))
            .sum()
    }

    fn indices(&self, drawn: &RunPolyline) -> &[u32] {
        self.source.polyline(drawn.polyline as usize)
    }

    fn placed(&self, point: Point3) -> Point3 {
        self.placement
            .map_or(point, |placement| placement.apply_point(point))
    }

    fn polyline_lines<'a>(&'a self, drawn: &'a RunPolyline) -> impl Iterator<Item = Line> + 'a {
        let points = self
            .indices(drawn)
            .iter()
            .filter_map(|index| self.source.point(*index));
        let mut along = 0.0_f64;
        points.clone().zip(points.skip(1)).map(move |(start, end)| {
            let stroke = drawn.stroke.at(along as f32);
            along += start.distance(end);
            Line {
                start: self.placed(start),
                end: self.placed(end),
                color: drawn.color,
                width: drawn.width,
                layer: self.layer,
                pick: drawn.pick,
                stroke,
            }
        })
    }
}

impl PartialEq for PolylineRun {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.source, &other.source)
            && self.placement == other.placement
            && self.layer == other.layer
            && self.polylines == other.polylines
    }
}

impl fmt::Debug for PolylineRun {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PolylineRun")
            .field("polylines", &self.polylines.len())
            .field("placement", &self.placement)
            .field("layer", &self.layer)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Stroke;

    struct Square {
        points: Vec<Point3>,
        polylines: Vec<Vec<u32>>,
    }

    impl PolylineSource for Square {
        fn polyline(&self, index: usize) -> &[u32] {
            self.polylines.get(index).map_or(&[], Vec::as_slice)
        }

        fn point(&self, index: u32) -> Option<Point3> {
            self.points.get(index as usize).copied()
        }
    }

    fn square() -> Arc<dyn PolylineSource> {
        Arc::new(Square {
            points: vec![
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(2.0, 0.0, 0.0),
                Point3::new(2.0, 2.0, 0.0),
                Point3::new(0.0, 2.0, 0.0),
            ],
            polylines: vec![vec![0, 1, 2], vec![2, 3, 0]],
        })
    }

    fn drawn(polyline: u32, stroke: StrokeKind) -> RunPolyline {
        RunPolyline {
            polyline,
            color: Color::from_rgb8(10, 20, 30),
            width: 1.5,
            pick: PickId::from_index(polyline as usize),
            stroke,
        }
    }

    #[test]
    fn a_styled_polyline_is_far_smaller_than_the_lines_it_draws() {
        assert_eq!(size_of::<RunPolyline>(), 32);
        assert_eq!(size_of::<Line>(), 88);
    }

    #[test]
    fn a_run_draws_each_polyline_as_placed_segments_dashed_along_its_own_length() {
        let run = PolylineRun {
            source: square(),
            placement: RigidTransform::translation(caditor_geometry::Vector3::new(0.0, 0.0, 5.0)),
            layer: Layer::Model,
            polylines: vec![
                drawn(0, StrokeKind::Solid),
                drawn(1, StrokeKind::DashedWhereHidden { seen_dashed: true }),
            ],
        };

        let lines: Vec<Line> = run.lines().collect();

        assert_eq!(run.segment_count(), 4);
        assert_eq!(lines.len(), 4);
        assert!(
            lines
                .iter()
                .all(|line| line.start.z == 5.0 && line.end.z == 5.0)
        );
        assert_eq!(lines[1].start, Point3::new(2.0, 0.0, 5.0));
        assert_eq!(lines[1].stroke, Stroke::Solid);
        assert_eq!(
            lines[3].stroke,
            Stroke::DashedWhereHidden {
                along: 2.0,
                seen_dashed: true
            }
        );
        assert_eq!(lines[3].pick, PickId::from_index(1));
    }

    #[test]
    fn runs_are_equal_only_over_the_same_source() {
        let source = square();
        let run = |source: &Arc<dyn PolylineSource>| PolylineRun {
            source: Arc::clone(source),
            placement: None,
            layer: Layer::Model,
            polylines: vec![drawn(0, StrokeKind::Dashed)],
        };

        assert_eq!(run(&source), run(&source));
        assert_ne!(run(&source), run(&square()));
    }
}
