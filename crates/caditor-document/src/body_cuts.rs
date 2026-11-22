use std::collections::{BTreeMap, BTreeSet};

use caditor_geometry::Plane;
use caditor_kernel::{
    EdgeId, Profile, ProfileCurve, ProfileError, Region, RegionKey, RegionReference,
    ResolvedRegions, Selection, Solid, resolve_regions,
};
use caditor_sketch::{BSpline, SplineKind, body_edge_curve, is_body_edge_curve};

use crate::{
    attachment::face_plane,
    projection::{Outline, edge_outline},
    recompute::SketchResult,
    solid::RegionChoice,
    tolerance::{self, POSITION_TOLERANCE},
};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BodyCuts {
    curves: Vec<ProfileCurve>,
}

impl BodyCuts {
    pub(crate) fn on(solid: &Solid, plane: &Plane) -> Self {
        let edges: BTreeSet<EdgeId> = solid
            .faces()
            .filter(|(face, _)| {
                face_plane(solid, *face).is_some_and(|found| lies_on(&found, plane))
            })
            .flat_map(|(_, face)| face.loops())
            .filter_map(|id| solid.face_loop(*id))
            .flat_map(|face_loop| face_loop.coedges())
            .filter_map(|coedge| solid.coedge(*coedge))
            .map(|coedge| coedge.edge())
            .collect();
        let mut named: BTreeMap<u64, Vec<EdgeId>> = BTreeMap::new();

        for edge in edges {
            if let Some(found) = solid.edge(edge) {
                named
                    .entry(body_edge_curve(found.name().digest()))
                    .or_default()
                    .push(edge);
            }
        }
        Self {
            curves: named
                .into_iter()
                .filter_map(|(id, edges)| match edges.as_slice() {
                    [edge] => edge_curve(solid, *edge, plane, id),
                    _ => None,
                })
                .collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.curves.is_empty()
    }

    pub fn curves(&self) -> &[ProfileCurve] {
        &self.curves
    }

    pub fn holds(&self, entity: u64) -> bool {
        self.curves
            .binary_search_by_key(&entity, |curve| curve.entity)
            .is_ok()
    }
}

fn lies_on(face: &Plane, plane: &Plane) -> bool {
    tolerance::parallel(face.normal(), plane.normal())
        && plane.signed_distance(face.origin()).abs() <= POSITION_TOLERANCE
}

fn edge_curve(solid: &Solid, edge: EdgeId, plane: &Plane, id: u64) -> Option<ProfileCurve> {
    Some(match edge_outline(solid, edge, plane)? {
        Outline::Point(_) => return None,
        Outline::Line { start, end } => ProfileCurve::line(id, start, end),
        Outline::Circle { center, radius } => ProfileCurve::circle(id, center, radius),
        Outline::Arc { center, start, end } => ProfileCurve::arc(id, center, start, end),
        Outline::Ellipse {
            center,
            major,
            minor_radius,
        } => ProfileCurve::ellipse(id, center, major - center, minor_radius),
        Outline::EllipticalArc {
            center,
            major,
            minor_radius,
            start,
            end,
        } => ProfileCurve::elliptical_arc(id, center, major - center, minor_radius, (start, end)),
        Outline::Spline { points, kind } if kind == SplineKind::OPEN => {
            let spline = BSpline::clamped(points)?;
            ProfileCurve::spline(
                id,
                spline.degree(),
                spline.knots().to_vec(),
                spline.control_points().to_vec(),
            )
        }
        Outline::Spline { .. } => return None,
    })
}

pub fn names_body_edges(reference: &RegionReference) -> bool {
    reference
        .boundary()
        .iter()
        .any(|piece| is_body_edge_curve(piece.entity))
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ChoiceError {
    Profile(ProfileError),
    EdgesGone,
}

impl From<ProfileError> for ChoiceError {
    fn from(error: ProfileError) -> Self {
        Self::Profile(error)
    }
}

pub(crate) struct ChosenRegions<'a> {
    pub(crate) profile: &'a Profile,
    pub(crate) resolved: ResolvedRegions,
    pub(crate) cutters: &'a [ProfileCurve],
    pieces: bool,
}

impl ChosenRegions<'_> {
    pub(crate) fn selection(&self) -> Selection {
        Selection::Regions(self.resolved.keys.clone())
    }

    pub(crate) fn regions(&self) -> Result<Vec<Region>, ProfileError> {
        self.profile.select(&self.selection())
    }
}

impl SketchResult {
    pub(crate) fn resolve_choice(
        &self,
        references: &[RegionReference],
    ) -> Result<ChosenRegions<'_>, ChoiceError> {
        if !references.iter().any(names_body_edges) {
            let profile = self.profile()?;

            return Ok(ChosenRegions {
                profile,
                resolved: resolve_regions(references, profile.regions())?,
                cutters: &[],
                pieces: false,
            });
        }

        let (live, gone): (Vec<RegionReference>, Vec<RegionReference>) =
            references.iter().cloned().partition(|reference| {
                !names_body_edges(reference)
                    || reference
                        .boundary()
                        .iter()
                        .any(|piece| self.cuts().holds(piece.entity))
            });

        if live.is_empty() {
            return Err(ChoiceError::EdgesGone);
        }

        let mut chosen = match self.cut()? {
            Some(cut) => ChosenRegions {
                profile: cut.profile(),
                resolved: resolve_regions(&live, cut.offered().map(|(region, _)| region))?,
                cutters: cut.cutters(),
                pieces: true,
            },
            None => {
                let profile = self.profile()?;
                ChosenRegions {
                    profile,
                    resolved: resolve_regions(&live, profile.regions())?,
                    cutters: &[],
                    pieces: false,
                }
            }
        };

        chosen
            .resolved
            .gone
            .extend(gone.iter().map(RegionReference::key));
        Ok(chosen)
    }

    pub fn chosen_keys(&self, choice: &RegionChoice) -> BTreeSet<RegionKey> {
        let Some(Ok(offered)) = self.regions() else {
            return BTreeSet::new();
        };
        let references = match choice {
            RegionChoice::All => {
                return offered
                    .iter()
                    .filter(|region| region.even_depth)
                    .map(|region| region.region.key())
                    .collect();
            }
            RegionChoice::Chosen(references) => references,
        };
        let Ok(chosen) = self.resolve_choice(references) else {
            return references.iter().map(RegionReference::key).collect();
        };
        offered
            .iter()
            .filter(|region| {
                let key = match chosen.pieces {
                    true => region.region.key(),
                    false => region.within,
                };
                chosen.resolved.keys.contains(&key)
            })
            .map(|region| region.region.key())
            .collect()
    }
}
