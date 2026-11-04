mod contour;
mod events;
mod flatten;
mod outline;
#[cfg(test)]
mod tests;
mod trace;
mod visibility;

use std::collections::BTreeMap;

use caditor_geometry::{Aabb, Plane, Point2, Point3, Vector3};
use thiserror::Error;

pub use self::outline::{OutlinePiece, outer_outline};
use self::{
    contour::{Contour, contours, facing, kept_ranges, roots_among, sample_parameters},
    events::add_crossings,
    flatten::{Flat, flat_parameter, flatten},
    visibility::{Verdict, verdict},
};
use crate::{
    blend::meets_smoothly,
    box_tree::BoxTree,
    curve::Curve,
    curve2::Curve2,
    error::GeometryError,
    interrupt::{self, Interrupted},
    intersect::{IntersectionError, intersect_curve_surface},
    interval::Interval,
    naming::{EdgeName, FaceName},
    surface::Surface,
    tally,
    tolerance::SamplingTolerance,
    topology::{EdgeId, FaceContainment, FaceId, Solid, SolidClassifier},
};

const SAME_SPLIT: f64 = 1e-7;
const PROBES: [f64; 3] = [0.5, 0.25, 0.75];

#[derive(Debug, Clone, PartialEq, Error)]
pub enum HiddenLineError {
    #[error("working out the hidden lines was cancelled")]
    Cancelled(#[from] Interrupted),
    #[error("a view needs a direction to look along")]
    NoLookDirection,
    #[error("a view's up direction must not lie along the direction it looks")]
    UpAlongLook,
    #[error("no frame could be placed for the outline of a curved face")]
    NoContourFrame,
    #[error("a curve of the view could not be made: {0}")]
    Geometry(#[from] GeometryError),
    #[error("whether a stretch of the outline stands against empty space could not be told")]
    OutlineUndecided,
    #[error("the pieces of the outline do not join up into closed loops")]
    OutlineOpen,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HiddenLineView {
    frame: Plane,
}

impl HiddenLineView {
    pub fn new(look: Vector3, up: Vector3) -> Result<Self, HiddenLineError> {
        let toward = (-look)
            .try_normalize()
            .ok_or(HiddenLineError::NoLookDirection)?;
        let right = up
            .cross(toward)
            .try_normalize()
            .ok_or(HiddenLineError::UpAlongLook)?;
        let frame =
            Plane::from_frame(Point3::ZERO, toward, right).ok_or(HiddenLineError::UpAlongLook)?;
        Ok(Self { frame })
    }

    pub fn square_to(plane: Plane) -> Self {
        Self { frame: plane }
    }

    pub fn frame(&self) -> &Plane {
        &self.frame
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Visible,
    Hidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewEdgeKind {
    Sharp,
    Tangent,
    Seam,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewSource {
    Edge {
        solid: usize,
        edge: EdgeId,
        name: EdgeName,
        kind: ViewEdgeKind,
    },
    Silhouette {
        solid: usize,
        face: FaceId,
        name: FaceName,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ViewPiece {
    pub curve: Curve2,
    pub range: Interval,
    pub visibility: Visibility,
    pub source: ViewSource,
    pub exact: bool,
}

impl ViewPiece {
    pub fn length(&self) -> f64 {
        self.curve.length(self.range)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HiddenLines {
    pub pieces: Vec<ViewPiece>,
    pub chord: f64,
    pub undecided: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Own {
    Edge(EdgeId),
    Contour(FaceId),
}

#[derive(Debug, Clone)]
struct Track {
    curve: Curve,
    flat: Flat,
    closed: bool,
    sampled: bool,
    source: ViewSource,
    own: Own,
    solid: usize,
    faces: Vec<FaceId>,
    samples: Vec<(f64, Point2)>,
    splits: Vec<f64>,
}

struct SolidView<'a> {
    solid: &'a Solid,
    classifier: SolidClassifier<'a>,
}

struct Scene<'a> {
    views: Vec<SolidView<'a>>,
    tree: BoxTree,
}

impl<'a> Scene<'a> {
    fn of(solids: &[&'a Solid]) -> Self {
        let views: Vec<SolidView<'a>> = solids
            .iter()
            .map(|solid| SolidView {
                solid,
                classifier: solid.classifier(),
            })
            .collect();
        let tree = BoxTree::new(views.iter().map(|view| {
            view.classifier
                .bounds()
                .unwrap_or_else(|| Aabb::from_point(Point3::ZERO))
        }));
        Self { views, tree }
    }
}

fn tracks_of_scene(
    scene: &Scene<'_>,
    plane: &Plane,
    tolerance: &SamplingTolerance,
) -> Result<(Vec<Track>, usize), HiddenLineError> {
    let build = Build {
        view: plane,
        tolerance,
    };
    let mut tracks = Vec::new();
    for (index, solid_view) in scene.views.iter().enumerate() {
        let mut boundary = BTreeMap::new();
        tracks.extend(build.edge_tracks(solid_view.solid, index, &mut boundary)?);
        tracks.extend(build.contour_tracks(solid_view, index, &boundary)?);
    }
    add_crossings(&mut tracks, plane, tolerance.chord())?;
    let failed = add_piercings(&mut tracks, scene)?;
    Ok((tracks, failed))
}

fn samples_of(
    curve: &Curve,
    range: Interval,
    view: &Plane,
    tolerance: &SamplingTolerance,
) -> Vec<(f64, Point2)> {
    match curve {
        Curve::BSpline(spline) if spline.degree() == 1 => {
            let inner = spline
                .breakpoints()
                .into_iter()
                .filter(|knot| *knot > range.start() && *knot < range.end());
            std::iter::once(range.start())
                .chain(inner)
                .chain(std::iter::once(range.end()))
                .map(|parameter| (parameter, view.to_local(curve.point(parameter))))
                .collect()
        }
        _ => curve
            .sample(range, tolerance)
            .iter()
            .map(|sample| (sample.parameter, view.to_local(sample.point)))
            .collect(),
    }
}

struct TrackPlan {
    closed: bool,
    sampled: bool,
    source: ViewSource,
    own: Own,
    solid: usize,
    faces: Vec<FaceId>,
    splits: Vec<f64>,
}

fn tracks_of(
    curve: &Curve,
    range: Interval,
    plan: &TrackPlan,
    view: &Plane,
    tolerance: &SamplingTolerance,
) -> Result<Vec<Track>, HiddenLineError> {
    let flats = flatten(curve, range, view, tolerance)?;
    let whole = flats.len() == 1;
    Ok(flats
        .into_iter()
        .map(|flat| {
            let splits = plan
                .splits
                .iter()
                .copied()
                .filter(|split| flat.range.contains(*split))
                .collect();
            Track {
                samples: samples_of(curve, flat.range, view, tolerance),
                closed: plan.closed && whole,
                sampled: plan.sampled || flat.sampled,
                curve: curve.clone(),
                flat,
                source: plan.source,
                own: plan.own,
                solid: plan.solid,
                faces: plan.faces.clone(),
                splits,
            }
        })
        .collect())
}

fn edge_kind(solid: &Solid, edge: EdgeId, faces: &[FaceId]) -> ViewEdgeKind {
    match faces {
        [first, second] if first == second => ViewEdgeKind::Seam,
        _ if meets_smoothly(solid, edge) => ViewEdgeKind::Tangent,
        _ => ViewEdgeKind::Sharp,
    }
}

fn is_curved(surface: &Surface) -> bool {
    !matches!(surface, Surface::Plane(_))
}

struct Build<'a> {
    view: &'a Plane,
    tolerance: &'a SamplingTolerance,
}

impl Build<'_> {
    fn edge_tracks(
        &self,
        solid: &Solid,
        index: usize,
        boundary: &mut BTreeMap<FaceId, Vec<Point3>>,
    ) -> Result<Vec<Track>, HiddenLineError> {
        let toward = self.view.normal();
        let mut tracks = Vec::new();
        for (id, edge) in solid.edges() {
            interrupt::check()?;
            let faces: Vec<FaceId> = edge
                .coedges()
                .iter()
                .filter_map(|coedge| solid.coedge_face(*coedge))
                .collect();
            let mut splits = Vec::new();
            let mut done: Vec<FaceId> = Vec::with_capacity(2);
            for coedge in edge.coedges() {
                let (Some(data), Some(face_id)) =
                    (solid.coedge(*coedge), solid.coedge_face(*coedge))
                else {
                    continue;
                };
                let Some(face) = solid.face(face_id) else {
                    continue;
                };
                if done.contains(&face_id) || !is_curved(face.surface()) {
                    continue;
                }
                done.push(face_id);
                let surface = face.surface();
                let parameters = sample_parameters(edge.curve(), edge.interval(), self.tolerance);
                let roots = roots_among(&parameters, |parameter| {
                    let hint = data.pcurve().uv_at(parameter);
                    let uv = surface.project(edge.curve().point(parameter), Some(hint));
                    facing(surface, uv, toward)
                });
                boundary
                    .entry(face_id)
                    .or_default()
                    .extend(roots.iter().map(|root| edge.curve().point(*root)));
                splits.extend(roots);
            }
            let plan = TrackPlan {
                closed: edge.is_closed(),
                sampled: false,
                source: ViewSource::Edge {
                    solid: index,
                    edge: id,
                    name: edge.name(),
                    kind: edge_kind(solid, id, &faces),
                },
                own: Own::Edge(id),
                solid: index,
                faces,
                splits,
            };
            tracks.extend(tracks_of(
                edge.curve(),
                edge.interval(),
                &plan,
                self.view,
                self.tolerance,
            )?);
        }
        Ok(tracks)
    }

    fn contour_tracks(
        &self,
        view: &SolidView<'_>,
        index: usize,
        boundary: &BTreeMap<FaceId, Vec<Point3>>,
    ) -> Result<Vec<Track>, HiddenLineError> {
        let mut tracks = Vec::new();
        for (id, face) in view.solid.faces() {
            interrupt::check()?;
            let surface = face.surface();
            let Some(uv_box) = view.classifier.face_uv_box(id) else {
                continue;
            };
            let points: &[Point3] = boundary.get(&id).map_or(&[], Vec::as_slice);
            for found in contours(surface, uv_box, self.view, self.tolerance)? {
                tracks.extend(self.contour_pieces(view, index, id, &found, points)?);
            }
        }
        Ok(tracks)
    }

    fn contour_pieces(
        &self,
        view: &SolidView<'_>,
        index: usize,
        id: FaceId,
        found: &Contour,
        points: &[Point3],
    ) -> Result<Vec<Track>, HiddenLineError> {
        let Some(face) = view.solid.face(id) else {
            return Ok(Vec::new());
        };
        let kept = kept_ranges(
            found,
            face.surface(),
            id,
            &view.classifier,
            points,
            self.tolerance.chord(),
        );
        let mut tracks = Vec::new();
        for range in kept {
            let plan = TrackPlan {
                closed: found.closed && range == found.range,
                sampled: found.sampled,
                source: ViewSource::Silhouette {
                    solid: index,
                    face: id,
                    name: face.name(),
                },
                own: Own::Contour(id),
                solid: index,
                faces: vec![id],
                splits: Vec::new(),
            };
            tracks.extend(tracks_of(
                &found.curve,
                range,
                &plan,
                self.view,
                self.tolerance,
            )?);
        }
        Ok(tracks)
    }
}

fn add_piercings(tracks: &mut [Track], scene: &Scene<'_>) -> Result<usize, HiddenLineError> {
    let mut failed = 0;
    if scene.views.len() < 2 {
        return Ok(failed);
    }
    for track in tracks.iter_mut() {
        interrupt::check()?;
        let box_of_track = track.curve.bounding_box(track.flat.range);
        for index in scene.tree.overlapping(&box_of_track, 0.0) {
            let Some(view) = scene.views.get(index).filter(|_| index != track.solid) else {
                continue;
            };
            for (face, uv_box) in view.classifier.faces_overlapping(&box_of_track) {
                let Some(definition) = view.solid.face(face) else {
                    continue;
                };
                tally::add(1);
                let found = match intersect_curve_surface(
                    &track.curve,
                    track.flat.range,
                    definition.surface(),
                    Some(uv_box),
                ) {
                    Ok(found) => found,
                    Err(IntersectionError::Cancelled(interrupted)) => {
                        return Err(interrupted.into());
                    }
                    Err(_) => {
                        failed += 1;
                        continue;
                    }
                };
                for hit in found.points {
                    let contained = view.classifier.point_in_face(face, hit.uv);
                    if contained.is_some_and(|class| class != FaceContainment::Outside) {
                        track.splits.push(hit.parameter);
                    }
                }
            }
        }
    }
    Ok(failed)
}

fn piece_ends(track: &Track, view: &Plane) -> Vec<f64> {
    let range = track.flat.range;
    let mut inner: Vec<f64> = track
        .splits
        .iter()
        .copied()
        .filter(|split| *split > range.start() && *split < range.end())
        .collect();
    inner.sort_by(f64::total_cmp);
    let flat_point = |parameter: f64| view.to_local(track.curve.point(parameter));
    let mut ends = vec![range.start()];
    let mut last = flat_point(range.start());
    for split in inner {
        let at = flat_point(split);
        if at.distance(last) > SAME_SPLIT {
            ends.push(split);
            last = at;
        }
    }
    if ends.len() > 1 && flat_point(range.end()).distance(last) <= SAME_SPLIT {
        ends.pop();
    }
    ends.push(range.end());
    ends
}

struct Classified {
    range: Interval,
    visibility: Visibility,
}

fn decided(
    track: &Track,
    range: Interval,
    scene: &Scene<'_>,
    toward: Vector3,
) -> Result<Option<Visibility>, HiddenLineError> {
    for probe in PROBES {
        match verdict(scene, track, range.at(probe), toward)? {
            Verdict::Visible => return Ok(Some(Visibility::Visible)),
            Verdict::Hidden => return Ok(Some(Visibility::Hidden)),
            Verdict::Undecided => {}
        }
    }
    Ok(None)
}

fn classified_pieces(
    track: &Track,
    scene: &Scene<'_>,
    view: &Plane,
    undecided: &mut usize,
) -> Result<Vec<Classified>, HiddenLineError> {
    let ends = piece_ends(track, view);
    let mut pieces: Vec<Classified> = Vec::with_capacity(ends.len());
    for pair in ends.windows(2) {
        let [start, end] = pair else {
            continue;
        };
        let Some(range) = Interval::new(*start, *end) else {
            continue;
        };
        let visibility = decided(track, range, scene, view.normal())?.unwrap_or_else(|| {
            *undecided += 1;
            Visibility::Visible
        });
        match pieces.last_mut() {
            Some(last) if last.visibility == visibility => {
                if let Some(joined) = Interval::new(last.range.start(), range.end()) {
                    last.range = joined;
                }
            }
            _ => pieces.push(Classified { range, visibility }),
        }
    }
    Ok(joined_round_the_seam(track, pieces))
}

fn joined_round_the_seam(track: &Track, mut pieces: Vec<Classified>) -> Vec<Classified> {
    let Some(period) = track.curve.period().filter(|_| track.closed) else {
        return pieces;
    };
    if pieces.len() < 2 {
        return pieces;
    }
    let same = match (pieces.first(), pieces.last()) {
        (Some(first), Some(last)) => first.visibility == last.visibility,
        _ => false,
    };
    if !same {
        return pieces;
    }
    let first = pieces.remove(0);
    if let Some(last) = pieces.last_mut()
        && let Some(joined) = Interval::new(last.range.start(), first.range.end() + period)
    {
        last.range = joined;
    }
    pieces
}

fn view_piece(track: &Track, piece: &Classified, view: &Plane) -> Option<ViewPiece> {
    let to_flat = |parameter: f64| flat_parameter(&track.flat, &track.curve, parameter, view);
    let (start, end) = (to_flat(piece.range.start()), to_flat(piece.range.end()));
    Some(ViewPiece {
        curve: track.flat.curve.clone(),
        range: Interval::new(start.min(end), start.max(end))?,
        visibility: piece.visibility,
        source: track.source,
        exact: !track.sampled,
    })
}

pub fn hidden_lines(
    solids: &[&Solid],
    view: &HiddenLineView,
    tolerance: &SamplingTolerance,
) -> Result<HiddenLines, HiddenLineError> {
    let plane = view.frame();
    let scene = Scene::of(solids);
    let (tracks, mut undecided) = tracks_of_scene(&scene, plane, tolerance)?;
    let mut pieces = Vec::new();
    for track in &tracks {
        for piece in classified_pieces(track, &scene, plane, &mut undecided)? {
            pieces.extend(view_piece(track, &piece, plane));
        }
    }
    Ok(HiddenLines {
        pieces,
        chord: tolerance.chord(),
        undecided,
    })
}
