use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use caditor_geometry::Point3;
use caditor_kernel::{
    EdgeId, EdgeNaming, EdgeReference, FaceId, FaceOrigin, FaceReference, LINEAR_RESOLUTION, Solid,
    VertexId, VertexName, vertex_names,
};

use crate::{
    document::{Document, Feature, FeatureId, FeatureKind},
    edit::{Edit, Transaction},
    healing::{ReferenceVisitor, visit, visit_projections},
    origins,
    paste::{
        CarriedParameters, Copying, FeaturePaste, PasteError, PasteOrigin, PasteRefusal,
        PickFailure, PickFinder, carry_values,
    },
    paste_sections::{Sections, Trial, follows_sections},
    recompute::{CancelToken, Evaluation, Evaluator, Recompute},
    solid::RegionChoice,
};

const SAME_PLACE: f64 = 100.0 * LINEAR_RESOLUTION;
const REBUILT_LABEL: &str = "Copied features";

pub struct PickSearch<'a> {
    pub evaluator: &'a dyn Evaluator,
    pub cancel: &'a CancelToken,
    pub progress: &'a dyn Fn(usize, usize),
    pub threads_cosmetic: bool,
}

impl PickSearch<'_> {
    fn as_shown(&self, document: Document) -> Document {
        if self.threads_cosmetic && document.has_modelled_threads() {
            document.threads_as_cosmetic()
        } else {
            document
        }
    }
}

pub fn copied_picks(features: &[Feature]) -> usize {
    let copied: BTreeSet<FeatureId> = features.iter().map(Feature::id).collect();
    features
        .iter()
        .filter(|feature| {
            let others = |id: &FeatureId| *id != feature.id() && copied.contains(id);
            feature.kind.origin_features().iter().any(others)
                || vertex_bodies(&feature.kind).iter().any(others)
                || !corner_followers(feature, features).is_empty()
        })
        .count()
}

pub(crate) fn corner_followers(feature: &Feature, features: &[Feature]) -> Vec<FeatureId> {
    let copied: BTreeSet<FeatureId> = features.iter().map(Feature::id).collect();
    let outside: BTreeSet<FeatureId> = vertex_bodies(&feature.kind)
        .into_iter()
        .map(FeatureId::maker)
        .filter(|maker| !copied.contains(maker))
        .collect();
    if outside.is_empty() {
        return Vec::new();
    }
    features
        .iter()
        .take_while(|earlier| earlier.id() != feature.id())
        .filter(|earlier| {
            earlier
                .kind
                .dependencies()
                .iter()
                .any(|used| outside.contains(&used.maker()))
        })
        .map(Feature::id)
        .collect()
}

pub(crate) fn vertex_bodies(kind: &FeatureKind) -> BTreeSet<FeatureId> {
    let mut walked = kind.clone();
    let mut corners = CornerBodies::default();
    visit(&mut walked, &mut corners);
    visit_projections(&mut walked, &mut corners);
    corners.bodies
}

#[derive(Default)]
struct CornerBodies {
    bodies: BTreeSet<FeatureId>,
}

impl ReferenceVisitor for CornerBodies {
    fn face(&mut self, _body: FeatureId, _face: &mut FaceReference, _what: &str) {}

    fn edges(
        &mut self,
        _body: FeatureId,
        _edges: &mut [EdgeReference],
        _what: &dyn Fn(usize) -> String,
    ) {
    }

    fn regions(&mut self, _sketch: FeatureId, _choice: &mut RegionChoice) {}

    fn vertex(&mut self, body: FeatureId, _vertex: &mut VertexName, _what: &str) {
        self.bodies.insert(body);
    }
}

impl Document {
    pub fn paste_features_finding_picks(
        &self,
        features: &[Feature],
        parameters: &CarriedParameters,
        origin: PasteOrigin,
        search: &PickSearch<'_>,
        cache: &mut Recompute,
    ) -> Result<FeaturePaste, PasteError> {
        let mut finder = Finder {
            search,
            features,
            parameters,
            origin,
            target: cache,
            source: None,
            found: 0,
            total: copied_picks(features),
        };
        (search.progress)(0, finder.total);
        let planned = self.plan_paste(features, parameters, origin, &mut finder)?;
        (search.progress)(finder.total, finder.total);
        Ok(planned)
    }
}

enum Source {
    Target,
    Rebuilt(Box<Evaluation>),
    Unreadable,
}

struct Finder<'a> {
    search: &'a PickSearch<'a>,
    features: &'a [Feature],
    parameters: &'a CarriedParameters,
    origin: PasteOrigin,
    target: &'a mut Recompute,
    source: Option<Source>,
    found: usize,
    total: usize,
}

impl Finder<'_> {
    fn cancelled(&self) -> Result<(), PickFailure> {
        if self.search.cancel.is_cancelled() {
            return Err(PickFailure::Cancelled);
        }
        Ok(())
    }

    fn prepare_source(&mut self, target: &Document) -> Result<(), PickFailure> {
        if self.source.is_some() {
            return Ok(());
        }
        let source = if self.origin == PasteOrigin::ThisDocument
            && originals_in_place(target, self.features)
        {
            Source::Target
        } else {
            match rebuilt(self.features, self.parameters) {
                Some(document) => {
                    let evaluation = self.target.draft_copy().run_without_display(
                        &self.search.as_shown(document),
                        self.search.evaluator,
                        self.search.cancel,
                    );
                    self.cancelled()?;
                    Source::Rebuilt(Box::new(evaluation))
                }
                None => Source::Unreadable,
            }
        };
        self.source = Some(source);
        Ok(())
    }
}

impl PickFinder for Finder<'_> {
    fn find(
        &mut self,
        target: &Document,
        so_far: &Transaction,
        feature: &Feature,
        kind: &mut FeatureKind,
        copying: &Copying<'_>,
    ) -> Result<(), PickFailure> {
        (self.search.progress)(self.found.min(self.total), self.total);
        self.cancelled()?;
        let mut scratch = target.clone();
        if scratch.apply(so_far.clone()).is_err() {
            return Err(PickFailure::Refused(PasteRefusal::CopiedGeometry {
                name: copying.first_deep_name(),
            }));
        }
        let shown = self.search.as_shown(scratch);
        let copies =
            self.target
                .run_without_display(&shown, self.search.evaluator, self.search.cancel);
        self.cancelled()?;
        self.prepare_source(target)?;
        let originals = match &self.source {
            Some(Source::Target) => &copies,
            Some(Source::Rebuilt(evaluation)) => evaluation.as_ref(),
            Some(Source::Unreadable) | None => {
                if copying.deep.is_empty() {
                    return Ok(());
                }
                return Err(PickFailure::Refused(PasteRefusal::PicksUnreadable {
                    name: copying.first_deep_name(),
                }));
            }
        };
        let mut mapper = Mapper {
            originals,
            copies: &copies,
            feature: feature.id(),
            copying,
            original_indexes: BTreeMap::new(),
            copy_indexes: BTreeMap::new(),
            failure: None,
        };
        visit(kind, &mut mapper);
        visit_projections(kind, &mut mapper);
        if let Some(reason) = mapper.failure {
            return Err(PickFailure::Refused(reason));
        }
        if let FeatureKind::Sketch(definition) = &*kind
            && follows_sections(definition, copying)
        {
            let Some(trial) = Trial::of(&shown, feature, kind) else {
                return Err(PickFailure::Refused(PasteRefusal::CopiedGeometry {
                    name: copying.first_deep_name(),
                }));
            };
            let evaluation = self.target.run_without_display(
                &trial.document,
                self.search.evaluator,
                self.search.cancel,
            );
            self.cancelled()?;
            let sections = Sections {
                originals,
                trial: &evaluation,
                trial_sketch: trial.sketch,
                original_sketch: feature.id(),
                copying,
            };
            sections.follow(kind).map_err(PickFailure::Refused)?;
        }
        let fresh: BTreeSet<FeatureId> = copying.renamed.values().copied().collect();
        let unmapped = kind
            .origin_features()
            .into_iter()
            .find(|id| copying.names.contains_key(id) && !fresh.contains(id));
        if let Some(unmapped) = unmapped {
            return Err(PickFailure::Refused(PasteRefusal::CopiedGeometry {
                name: copying.name_of(unmapped),
            }));
        }
        self.found += 1;
        Ok(())
    }
}

fn originals_in_place(target: &Document, features: &[Feature]) -> bool {
    features.iter().all(|feature| {
        target
            .feature(feature.id())
            .is_some_and(|original| original.kind == feature.kind)
            && !target.is_rolled_back(feature.id())
    })
}

fn rebuilt(features: &[Feature], parameters: &CarriedParameters) -> Option<Document> {
    let ids: BTreeSet<FeatureId> = features.iter().map(Feature::id).collect();
    let empty = Document::default();
    let mut edits = Vec::with_capacity(features.len());
    for (index, feature) in features.iter().enumerate() {
        if !feature.kind.dependencies().is_subset(&ids) {
            return None;
        }
        let mut copy = feature.clone();
        copy.kind.rename_features(&|id| id);
        carry_values(
            &mut copy.kind,
            &mut copy.appearance,
            parameters,
            &empty,
            PasteOrigin::Elsewhere,
        )
        .ok()?;
        copy.group = None;
        edits.push(Edit::InsertFeature {
            index,
            feature: Arc::new(copy),
        });
    }
    let mut document = Document::default();
    document
        .apply(Transaction::new(REBUILT_LABEL, edits))
        .ok()?;
    Some(document)
}

pub(crate) type Origins = Vec<Option<FaceOrigin>>;

pub(crate) struct Described<Id> {
    pub(crate) id: Id,
    pub(crate) key: Origins,
    pub(crate) around: Vec<Origins>,
    pub(crate) places: Vec<Point3>,
}

pub(crate) struct Index {
    pub(crate) faces: Vec<Described<FaceId>>,
    pub(crate) edges: Vec<Described<EdgeId>>,
    vertices: Vec<Described<VertexId>>,
    naming: EdgeNaming,
    vertex_names: BTreeMap<VertexId, VertexName>,
    vertex_features: BTreeMap<VertexId, BTreeSet<u64>>,
}

impl Index {
    pub(crate) fn of(solid: &Solid, rename: &dyn Fn(u64) -> u64) -> Self {
        let origin_of = |face: FaceId| {
            solid
                .face(face)
                .and_then(|definition| definition.origin())
                .map(|origin| origin.with_features(rename))
        };
        let origins_of = |faces: &BTreeSet<FaceId>| -> Origins {
            let mut origins: Origins = faces.iter().map(|face| origin_of(*face)).collect();
            origins.sort_unstable();
            origins
        };
        let point_of = |vertex: VertexId| solid.vertex(vertex).map(|found| found.point());
        let middle_of = |edge: EdgeId| {
            solid
                .edge(edge)
                .map(|found| found.curve().point(found.interval().middle()))
        };

        let edge_faces: BTreeMap<EdgeId, BTreeSet<FaceId>> = solid
            .edges()
            .map(|(id, edge)| {
                let faces = edge
                    .coedges()
                    .iter()
                    .filter_map(|coedge| solid.coedge_face(*coedge))
                    .collect();
                (id, faces)
            })
            .collect();
        let mut vertex_faces: BTreeMap<VertexId, BTreeSet<FaceId>> = BTreeMap::new();
        for (id, edge) in solid.edges() {
            let faces = edge_faces.get(&id).cloned().unwrap_or_default();
            for vertex in [edge.start(), edge.end()] {
                vertex_faces
                    .entry(vertex)
                    .or_default()
                    .extend(faces.iter().copied());
            }
        }
        let vertex_origins: BTreeMap<VertexId, Origins> = vertex_faces
            .iter()
            .map(|(vertex, faces)| (*vertex, origins_of(faces)))
            .collect();
        let vertex_features: BTreeMap<VertexId, BTreeSet<u64>> = vertex_faces
            .iter()
            .map(|(vertex, faces)| {
                let features = faces
                    .iter()
                    .filter_map(|face| solid.face(*face)?.origin())
                    .flat_map(|origin| origin.features())
                    .collect();
                (*vertex, features)
            })
            .collect();

        let faces = solid
            .faces()
            .map(|(id, face)| {
                let edges: Vec<EdgeId> = face
                    .loops()
                    .iter()
                    .filter_map(|face_loop| solid.face_loop(*face_loop))
                    .flat_map(|face_loop| face_loop.coedges())
                    .filter_map(|coedge| solid.coedge(*coedge))
                    .map(|coedge| coedge.edge())
                    .collect();
                let neighbours: BTreeSet<FaceId> = edges
                    .iter()
                    .filter_map(|edge| edge_faces.get(edge))
                    .flatten()
                    .copied()
                    .filter(|other| *other != id)
                    .collect();
                let places = edges
                    .iter()
                    .filter_map(|edge| middle_of(*edge))
                    .chain(
                        edges
                            .iter()
                            .filter_map(|edge| solid.edge(*edge))
                            .filter_map(|edge| point_of(edge.start())),
                    )
                    .collect();
                Described {
                    id,
                    key: vec![origin_of(id)],
                    around: vec![origins_of(&neighbours)],
                    places,
                }
            })
            .collect();
        let edges = solid
            .edges()
            .map(|(id, edge)| {
                let mut around: Vec<Origins> = [edge.start(), edge.end()]
                    .iter()
                    .map(|vertex| vertex_origins.get(vertex).cloned().unwrap_or_default())
                    .collect();
                around.sort_unstable();
                Described {
                    id,
                    key: edge_faces.get(&id).map(origins_of).unwrap_or_default(),
                    around,
                    places: [point_of(edge.start()), middle_of(id), point_of(edge.end())]
                        .into_iter()
                        .flatten()
                        .collect(),
                }
            })
            .collect();
        let vertices = solid
            .vertices()
            .map(|(id, vertex)| Described {
                id,
                key: vertex_origins.get(&id).cloned().unwrap_or_default(),
                around: Vec::new(),
                places: vec![vertex.point()],
            })
            .collect();
        Self {
            faces,
            edges,
            vertices,
            naming: EdgeNaming::new(solid),
            vertex_names: vertex_names(solid),
            vertex_features,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Unmatched {
    Missing,
    Ambiguous,
}

pub(crate) fn choose<Id: Copy + PartialEq>(
    original: Id,
    originals: &[Described<Id>],
    copies: &[Described<Id>],
) -> Result<Id, Unmatched> {
    let Some(original) = originals.iter().find(|described| described.id == original) else {
        return Err(Unmatched::Missing);
    };
    let alike: Vec<&Described<Id>> = originals
        .iter()
        .filter(|described| described.key == original.key)
        .collect();
    let candidates: Vec<&Described<Id>> = copies
        .iter()
        .filter(|described| described.key == original.key)
        .collect();
    if let ([_], [only]) = (alike.as_slice(), candidates.as_slice()) {
        return Ok(only.id);
    }

    let near = |described: &&&Described<Id>| described.around == original.around;
    let alike_near: Vec<&&Described<Id>> = alike.iter().filter(near).collect();
    let candidates_near: Vec<&&Described<Id>> = candidates.iter().filter(near).collect();
    if let ([_], [only]) = (alike_near.as_slice(), candidates_near.as_slice()) {
        return Ok(only.id);
    }

    let placed: Vec<Id> = candidates
        .iter()
        .filter(|described| same_places(&original.places, &described.places))
        .map(|described| described.id)
        .collect();
    match placed.as_slice() {
        [only] => Ok(*only),
        [] => Err(Unmatched::Missing),
        _ => Err(Unmatched::Ambiguous),
    }
}

fn same_places(first: &[Point3], second: &[Point3]) -> bool {
    let close = |a: &Point3, b: &Point3| a.distance(*b) <= SAME_PLACE;
    let covered = |from: &[Point3], to: &[Point3]| {
        from.iter()
            .all(|point| to.iter().any(|other| close(point, other)))
    };
    first.len() == second.len() && covered(first, second) && covered(second, first)
}

struct Mapper<'a> {
    originals: &'a Evaluation,
    copies: &'a Evaluation,
    feature: FeatureId,
    copying: &'a Copying<'a>,
    original_indexes: BTreeMap<FeatureId, Index>,
    copy_indexes: BTreeMap<FeatureId, Index>,
    failure: Option<PasteRefusal>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Corner {
    Anywhere,
    AroundCopiedFaces,
}

struct Solids<'a> {
    original: &'a Solid,
    copy: &'a Solid,
    name: String,
}

impl<'a> Mapper<'a> {
    fn original_body(&self, body: FeatureId) -> FeatureId {
        self.copying.original_of(body)
    }

    fn names_copied(&self, features: &BTreeSet<FeatureId>) -> bool {
        features
            .iter()
            .any(|feature| self.copying.names.contains_key(feature))
    }

    fn copy_name(&self, body: FeatureId, features: &BTreeSet<FeatureId>) -> String {
        let original = self.original_body(body);
        if self.copying.names.contains_key(&original) {
            return self.copying.name_of(original);
        }
        features
            .iter()
            .find(|feature| self.copying.names.contains_key(feature))
            .map_or_else(
                || self.copying.first_deep_name(),
                |feature| self.copying.name_of(*feature),
            )
    }

    fn solids(&mut self, body: FeatureId, name: String) -> Result<Solids<'a>, PasteRefusal> {
        let original_body = self.original_body(body);
        let Some(original) = self.originals.body_seen_by(self.feature, original_body) else {
            return Err(PasteRefusal::PicksUnreadable { name });
        };
        let Some(copy) = self.copies.body(body) else {
            return Err(PasteRefusal::CopyUncomputed { name });
        };
        let renamed = self.copying.renamed;
        let rename = |raw: u64| {
            renamed
                .get(&FeatureId::from_raw(raw))
                .map_or(raw, |copy| copy.raw())
        };
        self.original_indexes
            .entry(body)
            .or_insert_with(|| Index::of(original, &rename));
        self.copy_indexes
            .entry(body)
            .or_insert_with(|| Index::of(copy, &|raw| raw));
        Ok(Solids {
            original,
            copy,
            name,
        })
    }

    fn indexes(&self, body: FeatureId) -> Option<(&Index, &Index)> {
        Some((
            self.original_indexes.get(&body)?,
            self.copy_indexes.get(&body)?,
        ))
    }

    fn map_face(
        &mut self,
        body: FeatureId,
        face: &FaceReference,
        what: &str,
    ) -> Result<FaceReference, PasteRefusal> {
        let name = self.copy_name(body, &origins::of_face(face));
        let solids = self.solids(body, name)?;
        let unreadable = || PasteRefusal::PicksUnreadable {
            name: solids.name.clone(),
        };
        let (originals, copies) = self.indexes(body).ok_or_else(unreadable)?;
        let original = face.resolve(solids.original).map_err(|_| unreadable())?;
        let copy = choose(original, &originals.faces, &copies.faces)
            .map_err(|unmatched| refusal(unmatched, what, &solids.name))?;
        let captured = FaceReference::capture(solids.copy, copy)
            .ok_or_else(|| refusal(Unmatched::Missing, what, &solids.name))?;
        if captured.resolve(solids.copy) != Ok(copy) {
            return Err(refusal(Unmatched::Ambiguous, what, &solids.name));
        }
        Ok(captured)
    }

    fn map_edge(
        &mut self,
        body: FeatureId,
        edge: &EdgeReference,
        what: &str,
    ) -> Result<EdgeReference, PasteRefusal> {
        let name = self.copy_name(body, &origins::of_edge(edge));
        let solids = self.solids(body, name)?;
        let unreadable = || PasteRefusal::PicksUnreadable {
            name: solids.name.clone(),
        };
        let (originals, copies) = self.indexes(body).ok_or_else(unreadable)?;
        let original = edge
            .resolve_in(&originals.naming)
            .map_err(|_| unreadable())?;
        let copy = choose(original, &originals.edges, &copies.edges)
            .map_err(|unmatched| refusal(unmatched, what, &solids.name))?;
        let captured = EdgeReference::capture_in(&copies.naming, copy)
            .ok_or_else(|| refusal(Unmatched::Missing, what, &solids.name))?;
        if captured.resolve_in(&copies.naming) != Ok(copy) {
            return Err(refusal(Unmatched::Ambiguous, what, &solids.name));
        }
        Ok(captured)
    }

    fn map_vertex(
        &mut self,
        body: FeatureId,
        vertex: VertexName,
        what: &str,
        wanted: Corner,
    ) -> Result<Option<VertexName>, PasteRefusal> {
        let name = self.copy_name(body, &BTreeSet::new());
        let solids = self.solids(body, name)?;
        let unreadable = || PasteRefusal::PicksUnreadable {
            name: solids.name.clone(),
        };
        let (originals, copies) = self.indexes(body).ok_or_else(unreadable)?;
        let named = |names: &BTreeMap<VertexId, VertexName>, wanted: VertexName| {
            names
                .iter()
                .filter(|(_, name)| **name == wanted)
                .map(|(id, _)| *id)
                .collect::<Vec<VertexId>>()
        };
        let [original] = named(&originals.vertex_names, vertex)[..] else {
            return Err(unreadable());
        };
        if wanted == Corner::AroundCopiedFaces {
            let around = originals.vertex_features.get(&original);
            let touches_copy = around.is_some_and(|features| {
                features.iter().any(|feature| {
                    self.copying
                        .names
                        .contains_key(&FeatureId::from_raw(*feature))
                })
            });
            if !touches_copy {
                return Ok(None);
            }
        }
        let copy = choose(original, &originals.vertices, &copies.vertices)
            .map_err(|unmatched| refusal(unmatched, what, &solids.name))?;
        let renamed = copies
            .vertex_names
            .get(&copy)
            .copied()
            .ok_or_else(|| refusal(Unmatched::Missing, what, &solids.name))?;
        if named(&copies.vertex_names, renamed).len() != 1 {
            return Err(refusal(Unmatched::Ambiguous, what, &solids.name));
        }
        Ok(Some(renamed))
    }

    fn body_is_copied(&self, body: FeatureId) -> bool {
        self.copying.renamed.values().any(|copy| *copy == body)
    }
}

pub(crate) fn refusal(unmatched: Unmatched, what: &str, name: &str) -> PasteRefusal {
    let (what, name) = (what.to_owned(), name.to_owned());
    match unmatched {
        Unmatched::Missing => PasteRefusal::PickNotFound { what, name },
        Unmatched::Ambiguous => PasteRefusal::PickAmbiguous { what, name },
    }
}

impl ReferenceVisitor for Mapper<'_> {
    fn face(&mut self, body: FeatureId, face: &mut FaceReference, what: &str) {
        if self.failure.is_some() || !self.names_copied(&origins::of_face(face)) {
            return;
        }
        match self.map_face(body, face, what) {
            Ok(found) => *face = found,
            Err(reason) => self.failure = Some(reason),
        }
    }

    fn edges(
        &mut self,
        body: FeatureId,
        edges: &mut [EdgeReference],
        what: &dyn Fn(usize) -> String,
    ) {
        for (index, edge) in edges.iter_mut().enumerate() {
            if self.failure.is_some() || !self.names_copied(&origins::of_edge(edge)) {
                continue;
            }
            match self.map_edge(body, edge, &what(index)) {
                Ok(found) => *edge = found,
                Err(reason) => self.failure = Some(reason),
            }
        }
    }

    fn regions(&mut self, _sketch: FeatureId, _choice: &mut RegionChoice) {}

    fn vertex(&mut self, body: FeatureId, vertex: &mut VertexName, what: &str) {
        if self.failure.is_some() {
            return;
        }
        if self.body_is_copied(body) {
            match self.map_vertex(body, *vertex, what, Corner::Anywhere) {
                Ok(found) => *vertex = found.unwrap_or(*vertex),
                Err(reason) => self.failure = Some(reason),
            }
            return;
        }
        if let Ok(Some(found)) = self.map_vertex(body, *vertex, what, Corner::AroundCopiedFaces) {
            *vertex = found;
        }
    }
}
