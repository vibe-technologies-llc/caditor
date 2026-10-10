use caditor_geometry::{Plane, Point2, Vector2};
use caditor_render::{SectionPlane, is_cut_away, kept_span};

use crate::snap::Screen;

pub struct SectionedScreen<'a, S> {
    pub screen: S,
    pub plane: Plane,
    pub section: &'a [SectionPlane],
    pub slack: f64,
}

impl<S: Screen> Screen for SectionedScreen<'_, S> {
    fn to_screen(&self, point: Point2) -> Option<Vector2> {
        self.screen.to_screen(point)
    }

    fn to_sketch(&self, point: Vector2) -> Option<Point2> {
        self.screen.to_sketch(point)
    }

    fn is_shown(&self, point: Point2) -> bool {
        !is_cut_away(self.section, self.plane.to_world(point), self.slack)
    }

    fn kept(&self, from: Point2, to: Point2) -> Option<(Point2, Point2)> {
        let (low, high) = kept_span(
            self.section,
            self.plane.to_world(from),
            self.plane.to_world(to),
            self.slack,
        )?;
        Some((from.lerp(to, low), from.lerp(to, high)))
    }
}

#[cfg(test)]
mod tests {
    use caditor_geometry::{Point3, Vector3};
    use caditor_render::CutFace;

    use super::*;

    struct Flat;

    impl Screen for Flat {
        fn to_screen(&self, point: Point2) -> Option<Vector2> {
            Some(point)
        }
    }

    fn keeping_below(x: f64) -> Vec<SectionPlane> {
        let plane = Plane::new(Point3::new(x, 0.0, 0.0), Vector3::X).unwrap();
        vec![SectionPlane {
            plane,
            cut_face: CutFace::Hatched,
        }]
    }

    #[test]
    fn a_segment_is_clipped_where_the_section_cuts_it() {
        let section = keeping_below(5.0);
        let screen = SectionedScreen {
            screen: Flat,
            plane: Plane::XY,
            section: &section,
            slack: 0.0,
        };

        let kept = screen.kept(Point2::new(0.0, 0.0), Point2::new(10.0, 0.0));
        let gone = screen.kept(Point2::new(6.0, 0.0), Point2::new(10.0, 0.0));
        let whole = screen.kept(Point2::new(0.0, 0.0), Point2::new(4.0, 0.0));

        assert_eq!(kept, Some((Point2::new(0.0, 0.0), Point2::new(5.0, 0.0))));
        assert_eq!(gone, None);
        assert_eq!(whole, Some((Point2::new(0.0, 0.0), Point2::new(4.0, 0.0))));
        assert!(screen.is_shown(Point2::new(4.0, 0.0)));
        assert!(!screen.is_shown(Point2::new(6.0, 0.0)));
    }
}
