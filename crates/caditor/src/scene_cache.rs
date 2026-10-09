use std::{collections::BTreeSet, sync::Arc};

use caditor_document::FeatureId;
use caditor_geometry::{Plane, Point3};
use caditor_render::{Batch, SectionPlane, View};
use caditor_sketch::Faceting;

use crate::{
    comb::CombDrawing,
    display_style::DisplayStyle,
    drawing::Preview,
    editing::Context,
    faceting::FacetLevel,
    interference_panel::Mark,
    manipulator::Drawn,
    scene::{self, BuiltScene, Highlight, SketchShapes, Sources},
    scene_palette::Contrast,
    selection::{Pickable, Selection},
    view_aids::ViewAids,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Revisions {
    pub document: u64,
    pub evaluation: u64,
    pub sketches: u64,
    pub bodies: u64,
    pub style: DisplayStyle,
    pub aids: ViewAids,
    pub analysed: u64,
    pub contrast: Contrast,
}

pub struct SceneInputs<'a> {
    pub sources: &'a Sources<'a>,
    pub revisions: Revisions,
    pub context: Context,
    pub highlight: Highlight<'a>,
    pub view: Option<&'a View>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Overlay {
    pub contrast: Contrast,
    pub plane: Option<Plane>,
    pub previews: Vec<Preview>,
    pub measured: Option<[Point3; 2]>,
    pub problems: Vec<Point3>,
    pub interference: Vec<Mark>,
    pub comb: Option<Arc<CombDrawing>>,
    pub manipulator: Option<Drawn>,
}

impl Overlay {
    fn batch(&self) -> Option<Arc<Batch>> {
        let mut batch = Batch::default();
        if let Some(plane) = self.plane {
            for preview in &self.previews {
                scene::add_preview(&mut batch, self.contrast.palette(), plane, preview);
            }
        }
        if let Some([from, to]) = self.measured {
            scene::add_measurement(&mut batch, from, to);
        }
        for problem in &self.problems {
            scene::add_problem(&mut batch, *problem);
        }
        for mark in &self.interference {
            scene::add_interference(&mut batch, mark);
        }
        if let Some(comb) = &self.comb {
            comb.add_to(&mut batch, self.contrast.palette());
        }
        if let Some(manipulator) = &self.manipulator {
            manipulator.add_to(&mut batch, self.contrast.palette());
        }
        (!batch.is_empty()).then(|| Arc::new(batch))
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Content {
    revisions: Revisions,
    context: Context,
    level: FacetLevel,
}

#[derive(Debug, Clone, PartialEq)]
struct Highlighted {
    selection: Selection,
    hovered: Vec<Pickable>,
    chosen_rows: Vec<FeatureId>,
}

impl Highlighted {
    fn is(&self, highlight: &Highlight<'_>) -> bool {
        self.selection == *highlight.selection
            && self.hovered == highlight.hovered
            && self.chosen_rows == highlight.chosen_rows
    }
}

pub struct SceneCache {
    level: Option<FacetLevel>,
    content: Option<Content>,
    shapes: SketchShapes,
    highlighted: Option<Highlighted>,
    built: Option<BuiltScene>,
    base_batches: usize,
    overlay: Overlay,
    overlay_batch: Option<Arc<Batch>>,
    highlightable: Option<Vec<Pickable>>,
    section: Vec<SectionPlane>,
    generation: u64,
}

impl Default for SceneCache {
    fn default() -> Self {
        Self {
            level: None,
            content: None,
            shapes: SketchShapes::new(FacetLevel::WITHOUT_A_VIEW.faceting()),
            highlighted: None,
            built: None,
            base_batches: 0,
            overlay: Overlay::default(),
            overlay_batch: None,
            highlightable: None,
            section: Vec::new(),
            generation: 0,
        }
    }
}

impl SceneCache {
    pub fn update(&mut self, inputs: &SceneInputs<'_>) {
        let level = match inputs.view {
            Some(view) => FacetLevel::following(self.level, FacetLevel::wanted_chord(view)),
            None => self.level.unwrap_or(FacetLevel::WITHOUT_A_VIEW),
        };
        self.level = Some(level);
        let content = Content {
            revisions: inputs.revisions,
            context: inputs.context,
            level,
        };
        let new_content = self.content.as_ref() != Some(&content);
        if new_content {
            self.shapes = SketchShapes::new(scene::drawn_faceting(
                inputs.sources,
                inputs.context,
                level.faceting(),
            ));
            self.content = Some(content);
        }
        let highlighted = self
            .highlighted
            .as_ref()
            .is_some_and(|highlighted| highlighted.is(&inputs.highlight));
        if !new_content && highlighted && self.built.is_some() {
            return;
        }

        let mut built = scene::build(
            inputs.sources,
            &inputs.highlight,
            inputs.context,
            &mut self.shapes,
        );
        self.generation = self.generation.wrapping_add(1);
        built.generation = self.generation;
        self.base_batches = built.scene.batches.len();
        built.scene.batches.extend(self.overlay_batch.clone());
        built.scene.section.clone_from(&self.section);
        self.highlightable = None;
        self.built = Some(built);
        self.highlighted = Some(Highlighted {
            selection: inputs.highlight.selection.clone(),
            hovered: inputs.highlight.hovered.to_vec(),
            chosen_rows: inputs.highlight.chosen_rows.to_vec(),
        });
    }

    pub fn show(&mut self, overlay: Overlay) {
        if overlay == self.overlay {
            return;
        }
        self.overlay_batch = overlay.batch();
        self.overlay = overlay;
        if let Some(built) = &mut self.built {
            built.scene.batches.truncate(self.base_batches);
            built.scene.batches.extend(self.overlay_batch.clone());
        }
    }

    pub fn set_section(&mut self, section: Vec<SectionPlane>) {
        if section == self.section {
            return;
        }
        self.section = section;
        self.generation = self.generation.wrapping_add(1);
        if let Some(built) = &mut self.built {
            built.scene.section.clone_from(&self.section);
            built.generation = self.generation;
        }
    }

    pub fn section(&self) -> &[SectionPlane] {
        &self.section
    }

    pub fn built(&self) -> Option<&BuiltScene> {
        self.built.as_ref()
    }

    pub fn edited_plane(&self) -> Option<Plane> {
        self.built.as_ref()?.edited.map(|sketch| sketch.plane)
    }

    pub fn faceting(&self) -> Faceting {
        self.shapes.faceting()
    }

    pub fn level(&self) -> Option<FacetLevel> {
        self.level
    }

    pub fn has_pickables(&self) -> bool {
        self.built
            .as_ref()
            .is_some_and(|built| built.picks.pickables().next().is_some())
    }

    pub fn highlightable(&mut self) -> &[Pickable] {
        let built = &self.built;
        self.highlightable.get_or_insert_with(|| {
            built
                .as_ref()
                .map(|built| distinct(built.picks.pickables()))
                .unwrap_or_default()
        })
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }
}

fn distinct(pickables: impl Iterator<Item = Pickable>) -> Vec<Pickable> {
    let mut seen = BTreeSet::new();
    pickables
        .filter(|pickable| seen.insert(*pickable))
        .collect()
}
