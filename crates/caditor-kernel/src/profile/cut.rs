use std::collections::BTreeSet;

use caditor_geometry::{Aabb2, Point2};

use crate::{
    interrupt,
    profile::{Piece, Profile, ProfileCurve, ProfileError, Region, RegionKey, geometry::overlaps},
};

const PROBE_STEPS: [f64; 3] = [16.0, 4.0, 1.0];
const PROBED_PIECES: usize = 8;

#[derive(Clone, Debug)]
pub struct CutProfile {
    profile: Profile,
    cutters: Vec<ProfileCurve>,
    cutter_ids: BTreeSet<u64>,
    holders: Vec<Option<RegionKey>>,
}

impl CutProfile {
    pub fn new(
        plain: &Profile,
        curves: &[ProfileCurve],
        cutters: &[ProfileCurve],
    ) -> Result<Self, ProfileError> {
        let reach = plain
            .regions()
            .iter()
            .filter_map(Region::bounds)
            .reduce(Aabb2::union)
            .map(|bounds| bounds.expanded(plain.tolerance()));
        let near: Vec<ProfileCurve> = cutters
            .iter()
            .filter(|cutter| reach.is_some_and(|reach| reaches(cutter, &reach)))
            .cloned()
            .collect();
        let cutter_ids = near.iter().map(|cutter| cutter.entity).collect();

        let profile = Profile::new(&[curves, &near].concat())?;
        let mut holders = Vec::with_capacity(profile.regions().len());

        for region in profile.regions() {
            interrupt::check()?;
            holders.push(holder(region, profile.tolerance(), plain));
        }
        Ok(Self {
            profile,
            cutters: near,
            cutter_ids,
            holders,
        })
    }

    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    pub fn cutters(&self) -> &[ProfileCurve] {
        &self.cutters
    }

    pub fn offered(&self) -> impl Iterator<Item = (&Region, RegionKey)> + Clone {
        self.profile
            .regions()
            .iter()
            .zip(&self.holders)
            .filter_map(|(region, holder)| Some((region, (*holder)?)))
    }

    pub fn is_cut(&self) -> bool {
        self.offered()
            .any(|(region, _)| self.bounded_by_cutter(region))
    }

    pub fn bounded_by_cutter(&self, region: &Region) -> bool {
        region
            .pieces()
            .any(|piece| self.cutter_ids.contains(&piece.entity()))
    }
}

fn reaches(cutter: &ProfileCurve, reach: &Aabb2) -> bool {
    cutter
        .curve()
        .is_ok_and(|(curve, range)| overlaps(&curve.bounding_box(range), reach))
}

fn holder(region: &Region, tolerance: f64, plain: &Profile) -> Option<RegionKey> {
    let probe = probe(region, tolerance)?;
    plain
        .regions()
        .iter()
        .find(|candidate| candidate.contains(probe))
        .map(Region::key)
}

fn probe(region: &Region, tolerance: f64) -> Option<Point2> {
    region
        .pieces()
        .take(PROBED_PIECES)
        .flat_map(|piece| {
            PROBE_STEPS
                .iter()
                .filter_map(move |step| beside(piece, step * tolerance))
        })
        .find(|point| region.contains(*point))
}

fn beside(piece: &Piece, distance: f64) -> Option<Point2> {
    let derivatives = piece.curve().evaluate(piece.range().middle());
    let along = derivatives.first.try_normalize()?;
    let travel = if piece.is_reversed() { -along } else { along };
    Some(derivatives.point + travel.perp() * distance)
}
