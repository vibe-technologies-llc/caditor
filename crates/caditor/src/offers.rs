use caditor_document::{AxisReference, Datum, DatumAxis, FeatureId};

use crate::{
    blend_tools::{self, BlendError, EdgeSource},
    coil_tools::{self, CoilStart},
    combine_tools::{self, BodyPair, CombineError},
    datum_tools::{self, DatumFromSelectionError},
    delete_face_tools, draft_tools,
    editing::SketchEditing,
    emboss_tools::{self, EmbossError, EmbossSource},
    hole_tools::{self, HoleStart, HoleStartError},
    image_tools::{self, ImageTarget, ImageTargetError},
    loft_tools::{self, LoftStart},
    mate_tools::{self, MateSource},
    measure,
    mirror_tools::{self, FaceMirrorSource, MirrorSource, MirrorToolError},
    model::Model,
    move_face_tools, move_tools,
    offset_face_tools::{self, FaceToolError},
    pattern_tools::{self, PatternSource, PatternToolError, Shape},
    pipe_tools::{self, PipeStart},
    primitive_tools::{self, PrimitiveError, PrimitiveSource},
    rib_tools::{self, RibStart},
    scale_tools::{self, ScaleError},
    selection::{Pickable, Selection},
    shell_tools::{self, FaceSource, ShellError},
    sketch_pattern_tools::{self, SketchSourceError},
    sketch_placement::{self, SketchTarget, TargetError},
    solid_tools::{self, FaceProfile, FaceSweepError, Sweep},
    split_face_tools::{self, SplitFaceError, SplitFaceSource},
    split_tools::{self, SplitError, SplitSource},
    sweep_tools::{self, SweepStart},
    thread_tools::{self, ThreadSource, ThreadToolError},
    units::Units,
};

pub const MAX_DESCRIBED: usize = 12;

#[derive(Debug, Clone, PartialEq)]
struct Basis {
    selection: u64,
    tree: Vec<FeatureId>,
    rows: Vec<FeatureId>,
    revision: u64,
    evaluation: u64,
    units: Units,
    meshing: bool,
    edited: Option<FeatureId>,
    opened: Option<FeatureId>,
    masses: u64,
}

pub type Chosen<'a> = (&'a [FeatureId], &'a [FeatureId]);

#[derive(Debug, Clone, PartialEq)]
pub struct Offers {
    pub sketch_target: Result<SketchTarget, TargetError>,
    pub model_axes: Vec<(Pickable, AxisReference)>,
    pub datum_plane: Result<Datum, DatumFromSelectionError>,
    pub datum_axis: Result<DatumAxis, DatumFromSelectionError>,
    pub datum_point: Result<Datum, DatumFromSelectionError>,
    pub coordinate_system: Result<Datum, DatumFromSelectionError>,
    pub blend: Result<EdgeSource, BlendError>,
    pub extrude_faces: Option<Result<FaceProfile, FaceSweepError>>,
    pub revolve_faces: Option<Result<FaceProfile, FaceSweepError>>,
    pub sweep: Result<SweepStart, sweep_tools::SweepError>,
    pub pipe: Result<PipeStart, pipe_tools::PipeError>,
    pub loft: Result<LoftStart, loft_tools::LoftError>,
    pub coil: Result<CoilStart, coil_tools::CoilError>,
    pub helix: Result<CoilStart, coil_tools::CoilError>,
    pub rib: Result<RibStart, rib_tools::RibError>,
    pub shell: Result<FaceSource, ShellError>,
    pub offset_face: Result<FaceSource, FaceToolError>,
    pub delete_face: Result<FaceSource, FaceToolError>,
    pub emboss: Result<EmbossSource, EmbossError>,
    pub move_face: Result<FaceSource, FaceToolError>,
    pub draft: Result<FaceSource, FaceToolError>,
    pub split_face: Result<SplitFaceSource, SplitFaceError>,
    pub primitive: Result<PrimitiveSource, PrimitiveError>,
    pub combine: Result<BodyPair, CombineError>,
    pub movement: Result<FeatureId, move_tools::BodyChoiceError>,
    pub mirror: Result<MirrorSource, MirrorToolError>,
    pub mirror_faces: Result<FaceMirrorSource, MirrorToolError>,
    pub split: Result<SplitSource, SplitError>,
    pub scale: Result<FeatureId, ScaleError>,
    pub pattern: Result<PatternSource, PatternToolError>,
    pub curve_pattern: Result<PatternSource, SketchSourceError>,
    pub point_pattern: Result<PatternSource, SketchSourceError>,
    pub thread: Result<ThreadSource, ThreadToolError>,
    pub mate: Result<MateSource, mate_tools::MateError>,
    pub mate_angle: Result<MateSource, mate_tools::MateError>,
    pub hole: Result<HoleStart, HoleStartError>,
    pub reference_image: Result<ImageTarget, ImageTargetError>,
    pub described: Vec<String>,
    pub selected: usize,
    pub size: Option<String>,
}

fn size_of(model: &Model, selection: &Selection, tree: &[FeatureId]) -> Option<String> {
    match (selection.iter().collect::<Vec<_>>().as_slice(), tree) {
        ([only], _) => measure::size_text(model, *only),
        ([], [body]) => measure::body_size_text(model, *body),
        _ => None,
    }
}

impl Offers {
    fn of(
        model: &Model,
        selection: &Selection,
        editing: &SketchEditing,
        (tree, rows): Chosen<'_>,
    ) -> Self {
        let document = model.document();
        let evaluation = model.evaluation();
        let end = document.bar_index();
        let model_axes: Vec<(Pickable, AxisReference)> = selection
            .in_pick_order()
            .into_iter()
            .filter_map(|pickable| {
                Some((pickable, datum_tools::axis_reference(model, pickable, end)?))
            })
            .collect();
        let faces =
            |sweep| solid_tools::faces_to_sweep(model, selection, editing, sweep, &model_axes);
        Self {
            extrude_faces: faces(Sweep::Extrude),
            revolve_faces: faces(Sweep::Revolve),
            sweep: sweep_tools::source(model, selection, editing),
            pipe: pipe_tools::source(model, selection),
            loft: loft_tools::source(model, selection),
            coil: coil_tools::source(model, selection),
            helix: coil_tools::source(model, selection),
            rib: rib_tools::source(model, selection, (tree, rows)),
            sketch_target: sketch_placement::sketch_target(model, selection),
            pattern: pattern_tools::source(model, selection, tree, rows),
            curve_pattern: sketch_pattern_tools::source(
                model,
                selection,
                (tree, rows),
                Shape::Curve,
            ),
            point_pattern: sketch_pattern_tools::source(
                model,
                selection,
                (tree, rows),
                Shape::Points,
            ),
            model_axes,
            datum_plane: datum_tools::plane_from_selection(model, selection, end),
            datum_axis: datum_tools::axis_from_selection(model, selection, end),
            datum_point: datum_tools::point_from_selection(model, selection, end),
            coordinate_system: datum_tools::frame_from_selection(model, selection, end),
            blend: blend_tools::selected_edges(model, selection, tree),
            shell: shell_tools::selected_faces(model, selection),
            offset_face: offset_face_tools::selected_faces(model, selection),
            delete_face: delete_face_tools::selected_faces(model, selection),
            emboss: emboss_tools::source(model, selection),
            move_face: move_face_tools::selected_faces(model, selection),
            draft: draft_tools::selected_faces(model, selection),
            split_face: split_face_tools::source(model, selection),
            primitive: primitive_tools::source(model, selection),
            combine: combine_tools::selected_bodies(model, selection, tree),
            movement: move_tools::selected_body(model, selection, tree),
            mirror: mirror_tools::source(model, selection, (tree, rows)),
            mirror_faces: mirror_tools::faces_source(model, selection),
            split: split_tools::source(model, selection, tree),
            scale: scale_tools::selected_body(model, selection, tree),
            thread: thread_tools::selected_face(model, selection),
            mate: mate_tools::source(model, selection),
            mate_angle: mate_tools::angle_source(model, selection),
            hole: hole_tools::start(model, selection, editing),
            reference_image: image_tools::target(model, selection, editing),
            described: selection
                .iter()
                .take(MAX_DESCRIBED)
                .map(|pickable| pickable.describe(document, evaluation))
                .collect(),
            selected: selection.len(),
            size: size_of(model, selection, tree),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SelectionOffers {
    current: Option<(Basis, Offers)>,
    #[cfg(test)]
    computations: usize,
}

impl SelectionOffers {
    pub fn refresh(
        &mut self,
        model: &Model,
        selection: &Selection,
        editing: &SketchEditing,
        chosen: Chosen<'_>,
    ) -> &Offers {
        let (tree, rows) = chosen;
        let basis = Basis {
            selection: selection.generation(),
            tree: tree.to_vec(),
            rows: rows.to_vec(),
            revision: model.revision(),
            evaluation: model.evaluation_generation(),
            units: model.units(),
            meshing: model.bodies_pending(),
            edited: editing.feature(),
            opened: editing.solid(),
            masses: model.masses_measured(),
        };
        let (_, offers) = match self.current.take() {
            Some((known, offers)) if known == basis => self.current.insert((known, offers)),
            Some(_) | None => {
                #[cfg(test)]
                {
                    self.computations += 1;
                }
                self.current
                    .insert((basis, Offers::of(model, selection, editing, chosen)))
            }
        };
        offers
    }

    #[cfg(test)]
    pub fn computations(&self) -> usize {
        self.computations
    }
}
