use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
};

use caditor_geometry::{Aabb, Point3, Vector3};

use crate::{
    boolean::{
        BooleanError, BooleanOperation, FaceKey, Input, Operand, TOLERANCE,
        faces::SplitFace,
        imprint::Arrangement,
        trace::{Fragment, deepest_points, interior_points},
    },
    interrupt,
    intersect::boxes_overlap,
    naming::{FaceName, FaceOrigin},
    sense::Sense,
    surface::Surface,
    tally,
    topology::{BoundaryClass, FaceId, PointClass, SolidClassifier},
};

const LYING_ALONG_SINE: f64 = 1e-2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Twin {
    Alone,
    Matched,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Class {
    Inside,
    Outside,
    Coincident(Sense),
}

#[derive(Clone, Debug)]
pub(super) struct KeptFace {
    pub key: FaceKey,
    pub surface: Surface,
    pub sense: Sense,
    pub name: FaceName,
    pub origin: Option<FaceOrigin>,
    pub fragment: Fragment,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Keep {
    No,
    AsIs,
    Reversed,
}

fn keep(operand: Operand, class: Class, operation: BooleanOperation) -> Keep {
    use BooleanOperation::{Difference, Intersection, Union};
    match (operand, class, operation) {
        (Operand::First, Class::Outside, Union | Difference)
        | (Operand::First, Class::Inside, Intersection)
        | (Operand::First, Class::Coincident(Sense::Same), Union | Intersection)
        | (Operand::First, Class::Coincident(Sense::Reversed), Difference)
        | (Operand::Second, Class::Outside, Union)
        | (Operand::Second, Class::Inside, Intersection) => Keep::AsIs,
        (Operand::Second, Class::Inside, Difference) => Keep::Reversed,
        _ => Keep::No,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Basis {
    Sampled,
    LyingAlong,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Verdict {
    class: Class,
    basis: Basis,
}

impl Verdict {
    fn sampled(class: Class) -> Self {
        Self {
            class,
            basis: Basis::Sampled,
        }
    }
}

pub(super) fn classify(
    input: &Input,
    key: FaceKey,
    surface: &Surface,
    sense: Sense,
    fragment: &Fragment,
    twin: Twin,
) -> Result<Class, BooleanError> {
    judge(input, key, surface, sense, fragment, twin).map(|verdict| verdict.class)
}

fn judge(
    input: &Input,
    key: FaceKey,
    surface: &Surface,
    sense: Sense,
    fragment: &Fragment,
    twin: Twin,
) -> Result<Verdict, BooleanError> {
    tally::add_judged_fragment();

    let other = key.operand.other();
    let classifier = input.classifier(other);
    let mut solid_side = None;
    let mut coincident: Option<Class> = None;
    let mut along_boundary: Option<Class> = None;
    let mut lying: Vec<Class> = Vec::new();
    let mut sampled = 0;
    let mut mixed_at = None;
    let mut coincident_at = None;
    let mut first_sample = None;

    for uv in interior_points(fragment, surface) {
        let Some(normal) = surface.normal(uv.x, uv.y) else {
            continue;
        };
        let point = surface.point_at(uv);

        first_sample.get_or_insert(point);
        sampled += 1;

        let ambiguous = || BooleanError::Ambiguous(Box::default()).or_point(|| Some(point));
        let outward = normal * sense.sign();
        let found = classifier.classify_fragment_point(point, outward, surface);
        match found {
            BoundaryClass::Inside | BoundaryClass::Outside => {
                let class = if matches!(found, BoundaryClass::Inside) {
                    Class::Inside
                } else {
                    Class::Outside
                };

                if solid_side.is_some_and(|known| known != class) {
                    return Err(ambiguous().or_faces([key]));
                }
                solid_side = Some(class);
            }
            BoundaryClass::Coincident { face, sense } => {
                let class = Class::Coincident(sense);

                if !exactly_inside(input, other, face, point) {
                    along_boundary.get_or_insert(class);
                    continue;
                }
                if coincident.is_some_and(|known| known != class) {
                    mixed_at.get_or_insert(point);
                }
                coincident_at.get_or_insert(point);
                coincident = Some(class);
            }
            BoundaryClass::Touching(face) => {
                if let Some(class) = lying_along(input, other, face, point, outward) {
                    lying.push(class);
                }
            }
            BoundaryClass::Undecided => {}
        }
    }

    let ambiguous = |point: Option<Point3>| {
        BooleanError::Ambiguous(Box::default())
            .or_faces([key])
            .or_point(|| point)
    };

    if let Some(point) = mixed_at.or(coincident_at.filter(|_| solid_side.is_some())) {
        return Err(ambiguous(Some(point)));
    }

    let lying_throughout = (lying.len() == sampled).then(|| agreed(&lying)).flatten();
    let lying_on_twin = lying_throughout.filter(|_| twin == Twin::Matched);
    let sampled = solid_side
        .or(coincident)
        .or(lying_on_twin)
        .or_else(|| classify_thin(classifier, surface, fragment))
        .or(along_boundary)
        .map(|class| Verdict {
            class,
            basis: Basis::Sampled,
        });
    let lying = lying_throughout.map(|class| Verdict {
        class,
        basis: Basis::LyingAlong,
    });
    sampled.or(lying).ok_or_else(|| ambiguous(first_sample))
}

fn lying_along(
    input: &Input,
    operand: Operand,
    face: FaceId,
    point: Point3,
    outward: Vector3,
) -> Option<Class> {
    let touched = input.solid(operand).face(face)?;
    let hint = input
        .bounds(FaceKey { operand, face })
        .map(|bounds| bounds.uv.center());

    let uv = touched.surface().project(point, hint);
    let normal = touched.surface().normal(uv.x, uv.y)? * touched.sense().sign();
    let along = normal.cross(outward.try_normalize()?).length() <= LYING_ALONG_SINE;
    let inside = input.classifier(operand).exactly_inside_face(face, uv) == Some(true);
    (along && inside).then(|| Class::Coincident(Sense::from_sign(normal.dot(outward))))
}

fn boundary_pieces(fragment: &Fragment) -> BTreeSet<usize> {
    fragment
        .loops
        .iter()
        .flat_map(|traced| &traced.coedges)
        .map(|coedge| coedge.half_edge.piece)
        .collect()
}

fn outlines(split: &[SplitFace], operand: Operand) -> BTreeSet<BTreeSet<usize>> {
    split
        .iter()
        .filter(|face| face.key.operand == operand)
        .flat_map(|face| &face.fragments)
        .map(boundary_pieces)
        .collect()
}

fn agreed(classes: &[Class]) -> Option<Class> {
    let (first, rest) = classes.split_first()?;
    rest.iter().all(|class| class == first).then_some(*first)
}

fn exactly_inside(input: &Input, operand: Operand, face: FaceId, point: Point3) -> bool {
    let Some(surface) = input.solid(operand).face(face).map(|face| face.surface()) else {
        return false;
    };
    let hint = input
        .bounds(FaceKey { operand, face })
        .map(|bounds| bounds.uv.center());

    let uv = surface.project(point, hint);
    input.classifier(operand).exactly_inside_face(face, uv) == Some(true)
}

fn classify_thin(
    classifier: &SolidClassifier,
    surface: &Surface,
    fragment: &Fragment,
) -> Option<Class> {
    deepest_points(fragment, surface)
        .into_iter()
        .find_map(
            |uv| match classifier.side_of_touched_faces(surface.point_at(uv)) {
                PointClass::Inside => Some(Class::Inside),
                PointClass::Outside => Some(Class::Outside),
                PointClass::OnBoundary(_) | PointClass::Undecided => None,
            },
        )
}

#[derive(Default)]
struct Components {
    parents: BTreeMap<FaceKey, FaceKey>,
    sizes: BTreeMap<FaceKey, usize>,
}

impl Components {
    fn root(&mut self, key: FaceKey) -> FaceKey {
        let mut root = key;

        while let Some(parent) = self.parents.get(&root)
            && *parent != root
        {
            root = *parent;
        }

        let mut walked = key;

        while walked != root {
            let Some(parent) = self.parents.insert(walked, root) else {
                break;
            };

            walked = parent;
        }
        root
    }

    fn size(&self, root: FaceKey) -> usize {
        self.sizes.get(&root).copied().unwrap_or(1)
    }

    fn join(&mut self, first: FaceKey, second: FaceKey) {
        let (first, second) = (self.root(first), self.root(second));

        if first == second {
            return;
        }

        let (small, large) = match self.size(first).cmp(&self.size(second)) {
            Ordering::Less => (first, second),
            Ordering::Greater => (second, first),
            Ordering::Equal => (first.max(second), first.min(second)),
        };

        let joined = self.size(small) + self.size(large);

        self.parents.insert(small, large);
        self.sizes.insert(large, joined);
    }
}

fn untouched_components(
    input: &Input,
    arrangement: &Arrangement,
    split: &[SplitFace],
) -> Components {
    let untouched: BTreeSet<FaceKey> = split
        .iter()
        .filter(|face| face.untouched)
        .map(|face| face.key)
        .collect();
    let shared = arrangement.shared_pieces();
    let mut components = Components::default();

    for operand in Operand::BOTH {
        let solid = input.solid(operand);
        for (id, edge) in solid.edges() {
            let [piece] = arrangement.edge_pieces(operand, id) else {
                continue;
            };

            if shared.contains(piece) {
                continue;
            }

            let faces: Vec<FaceKey> = edge
                .coedges()
                .iter()
                .filter_map(|coedge| solid.coedge_face(*coedge))
                .map(|face| FaceKey { operand, face })
                .collect();
            if let [first, second] = faces.as_slice()
                && untouched.contains(first)
                && untouched.contains(second)
            {
                components.join(*first, *second);
            }
        }
    }
    components
}

fn extent(input: &Input, operand: Operand) -> Option<Aabb> {
    input
        .faces(operand)
        .iter()
        .map(|face| face.bounds)
        .reduce(Aabb::union)
}

fn reaches_beyond(arrangement: &Arrangement, fragment: &Fragment, other: Option<&Aabb>) -> bool {
    fragment
        .loops
        .iter()
        .flat_map(|traced| &traced.coedges)
        .filter_map(|coedge| arrangement.curve(coedge.half_edge.piece))
        .any(|(curve, piece)| {
            other.is_none_or(|other| {
                !boxes_overlap(&curve.bounding_box(piece.interval), other, TOLERANCE)
            })
        })
}

fn apart(input: &Input, key: FaceKey, other: Option<&Aabb>) -> bool {
    let Some(other) = other else {
        return true;
    };
    input
        .bounds(key)
        .is_some_and(|bounds| !boxes_overlap(&bounds.bounds, other, TOLERANCE))
}

fn own_unshared(
    arrangement: &Arrangement,
    operand: Operand,
    shared: &BTreeSet<usize>,
    piece: usize,
) -> bool {
    !shared.contains(&piece)
        && arrangement
            .piece(piece)
            .and_then(|data| arrangement.source(data.source))
            .and_then(|source| source.edge)
            .is_some_and(|(owner, _)| owner == operand)
}

fn seam_neighbour(
    arrangement: &Arrangement,
    operand: Operand,
    shared: &BTreeSet<usize>,
    outlines: &[BTreeSet<usize>],
    verdicts: &[Verdict],
    index: usize,
) -> Option<Class> {
    let mine = outlines.get(index)?;
    outlines
        .iter()
        .zip(verdicts)
        .enumerate()
        .find(|(other, (theirs, verdict))| {
            *other != index
                && verdict.basis == Basis::Sampled
                && mine
                    .intersection(theirs)
                    .any(|piece| own_unshared(arrangement, operand, shared, *piece))
        })
        .map(|(_, (_, verdict))| verdict.class)
}

fn settle_across_seams(
    arrangement: &Arrangement,
    operand: Operand,
    shared: &BTreeSet<usize>,
    judged: &mut [(Fragment, Verdict)],
) {
    let outlines: Vec<BTreeSet<usize>> = judged
        .iter()
        .map(|(fragment, _)| boundary_pieces(fragment))
        .collect();
    let mut verdicts: Vec<Verdict> = judged.iter().map(|(_, verdict)| *verdict).collect();

    for _ in 0..verdicts.len() {
        let mut settled_any = false;

        for index in 0..verdicts.len() {
            let lying = verdicts
                .get(index)
                .is_some_and(|verdict| verdict.basis == Basis::LyingAlong);
            let neighbour = lying
                .then(|| seam_neighbour(arrangement, operand, shared, &outlines, &verdicts, index))
                .flatten();
            if let (Some(class), Some(slot)) = (neighbour, verdicts.get_mut(index)) {
                *slot = Verdict::sampled(class);
                settled_any = true;
            }
        }
        if !settled_any {
            break;
        }
    }
    for ((_, verdict), settled) in judged.iter_mut().zip(verdicts) {
        *verdict = settled;
    }
}

struct Regions {
    parents: Vec<usize>,
    classes: Vec<Option<Class>>,
}

impl Regions {
    fn of(
        input: &Input,
        arrangement: &Arrangement,
        shared: &BTreeSet<usize>,
        split: &[SplitFace],
    ) -> Self {
        let count = split.iter().map(|face| face.fragments.len()).sum();
        let mut regions = Self {
            parents: (0..count).collect(),
            classes: vec![None; count],
        };

        if !input.carrying {
            return regions;
        }

        let mut first_user: BTreeMap<usize, usize> = BTreeMap::new();
        let fragments = split.iter().flat_map(|face| {
            face.fragments
                .iter()
                .map(|fragment| (face.key.operand, fragment))
        });

        for (index, (operand, fragment)) in fragments.enumerate() {
            for piece in boundary_pieces(fragment) {
                if !own_unshared(arrangement, operand, shared, piece) {
                    continue;
                }
                match first_user.get(&piece) {
                    Some(user) => regions.join(*user, index),
                    None => {
                        first_user.insert(piece, index);
                    }
                }
            }
        }
        regions
    }

    fn root(&mut self, mut index: usize) -> usize {
        while let Some(parent) = self.parents.get(index).copied()
            && parent != index
        {
            let grandparent = self.parents.get(parent).copied().unwrap_or(parent);

            if let Some(slot) = self.parents.get_mut(index) {
                *slot = grandparent;
            }
            index = grandparent;
        }
        index
    }

    fn join(&mut self, first: usize, second: usize) {
        let (first, second) = (self.root(first), self.root(second));

        if let Some(slot) = self.parents.get_mut(first.max(second)) {
            *slot = first.min(second);
        }
    }

    fn class(&self, root: usize) -> Option<Class> {
        self.classes.get(root).copied().flatten()
    }

    fn learn(&mut self, root: usize, verdict: Verdict) {
        let sided = matches!(verdict.class, Class::Inside | Class::Outside);

        if let Some(slot) = self.classes.get_mut(root)
            && slot.is_none()
            && sided
            && verdict.basis == Basis::Sampled
        {
            *slot = Some(verdict.class);
        }
    }
}

pub(super) fn select(
    input: &Input,
    arrangement: &Arrangement,
    split: Vec<SplitFace>,
    operation: BooleanOperation,
) -> Result<Vec<KeptFace>, BooleanError> {
    let mut components = untouched_components(input, arrangement, &split);
    let extents = Operand::BOTH.map(|operand| extent(input, operand));
    let [first_outlines, second_outlines] = Operand::BOTH.map(|operand| outlines(&split, operand));
    let shared = arrangement.shared_pieces();
    let mut regions = Regions::of(input, arrangement, &shared, &split);
    let mut index = 0;
    let mut shared_classes: BTreeMap<FaceKey, Class> = BTreeMap::new();
    let mut kept = Vec::new();

    for face in split {
        interrupt::check()?;

        let original = input
            .face(face.key)
            .ok_or_else(|| BooleanError::split().or_faces([face.key]))?;
        let root = components.root(face.key);
        let [first_extent, second_extent] = &extents;

        let other_extent = match face.key.operand {
            Operand::First => second_extent.as_ref(),
            Operand::Second => first_extent.as_ref(),
        };
        let mut judged = Vec::with_capacity(face.fragments.len());

        for fragment in face.fragments {
            let region = regions.root(index);

            index += 1;

            let known = if face.untouched {
                shared_classes.get(&root).copied()
            } else {
                None
            };
            let verdict = match known {
                Some(class) => Verdict::sampled(class),
                None if face.untouched && apart(input, face.key, other_extent) => {
                    Verdict::sampled(Class::Outside)
                }
                None if input.carrying && reaches_beyond(arrangement, &fragment, other_extent) => {
                    Verdict::sampled(Class::Outside)
                }
                None if let Some(class) = regions.class(region) => Verdict::sampled(class),
                None => {
                    let others = match face.key.operand {
                        Operand::First => &second_outlines,
                        Operand::Second => &first_outlines,
                    };
                    let twin = if others.contains(&boundary_pieces(&fragment)) {
                        Twin::Matched
                    } else {
                        Twin::Alone
                    };
                    judge(
                        input,
                        face.key,
                        original.surface(),
                        original.sense(),
                        &fragment,
                        twin,
                    )?
                }
            };

            if face.untouched && !matches!(verdict.class, Class::Coincident(_)) {
                shared_classes.insert(root, verdict.class);
            }
            regions.learn(region, verdict);
            judged.push((fragment, verdict));
        }
        settle_across_seams(arrangement, face.key.operand, &shared, &mut judged);
        for (fragment, verdict) in judged {
            let (sense, fragment) = match keep(face.key.operand, verdict.class, operation) {
                Keep::No => continue,
                Keep::AsIs => (original.sense(), fragment),
                Keep::Reversed => (original.sense().reversed(), fragment.reversed()),
            };

            kept.push(KeptFace {
                key: face.key,
                surface: original.surface().clone(),
                sense,
                name: original.name(),
                origin: original.origin(),
                fragment,
            });
        }
    }
    Ok(kept)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(index: usize) -> FaceKey {
        FaceKey {
            operand: Operand::First,
            face: FaceId::from_index(index).unwrap(),
        }
    }

    fn depth(components: &Components, mut face: FaceKey) -> usize {
        let mut steps = 0;

        while let Some(parent) = components.parents.get(&face)
            && *parent != face
        {
            face = *parent;
            steps += 1;
        }
        steps
    }

    fn chain_stays_shallow(order: impl Iterator<Item = usize>, length: usize) {
        let mut components = Components::default();

        for index in order {
            components.join(key(index), key(index + 1));
        }

        let root = components.root(key(0));

        for index in 0..=length {
            assert_eq!(components.root(key(index)), root);
        }

        let deepest = (0..=length)
            .map(|index| depth(&components, key(index)))
            .max()
            .unwrap();

        assert!(deepest <= 1, "{deepest}");
    }

    #[test]
    fn joins_in_either_chain_order_end_in_one_flat_component() {
        chain_stays_shallow(0..4096, 4096);
        chain_stays_shallow((0..4096).rev(), 4096);
    }

    #[test]
    fn joins_stay_shallow_before_any_lookup() {
        let mut components = Components::default();

        for index in 0..4096 {
            components.join(key(index), key(index + 1));
        }

        let deepest = (0..=4096)
            .map(|index| depth(&components, key(index)))
            .max()
            .unwrap();

        assert!(deepest <= 13, "{deepest}");
    }

    #[test]
    fn separate_components_keep_separate_roots() {
        let mut components = Components::default();

        components.join(key(0), key(1));
        components.join(key(2), key(3));

        assert_eq!(components.root(key(0)), components.root(key(1)));
        assert_eq!(components.root(key(2)), components.root(key(3)));
        assert_ne!(components.root(key(0)), components.root(key(2)));
        assert_eq!(components.root(key(9)), key(9));
    }
}
