use caditor_document::FeatureId;
use caditor_geometry::Point2;
use caditor_sketch::{Entity, EntityId, Faceting, MirrorError, MirrorImage, Sketch};

use crate::{
    drawing::Preview,
    editing::Tool,
    feature_tree::count,
    model::Model,
    modifying::{Hint, Outcome, Prompt},
    snap::{self, Pointer, Screen},
    trimming::{self, capitalized},
};

pub const PROMPT: &str = "Click the line or axis to mirror the selection about";
pub const SELECT_FIRST: &str =
    "Select what to mirror: click it or drag a box around it, then press Enter";
pub const SELECT_MORE: &str = "Click or drag a box to add to what is mirrored, then press Enter";
pub const TRANSACTION: &str = "Mirror geometry";
const GATHER_KEYS: &str = "Click: add or remove   Drag: add what the box takes   Enter: choose the line to mirror about   Esc: back to Select";
const GATHERED_KEYS: &str = "Click: add or remove   Drag: add what the box takes   Enter: choose the line to mirror about   Esc: let go of the selection";
const LINE_KEYS: &str = "Esc: change what is mirrored";
const NOTHING_SELECTED: &str = "select what to mirror first: click it or drag a box around it";
const NO_LINE_HIGHLIGHTED: &str =
    "Highlight a line or an axis first, with Highlight the next item in the view";

#[derive(Debug, Clone, Default)]
pub struct Mirroring {
    selected: Vec<EntityId>,
    gathering: bool,
    gathered: bool,
    hover: Option<EntityId>,
    highlight: Option<EntityId>,
    image: Option<(EntityId, Result<MirrorImage, MirrorError>)>,
}

impl Mirroring {
    pub fn starting(selected: &[EntityId]) -> Self {
        Self {
            selected: selected.to_vec(),
            gathering: selected.is_empty(),
            gathered: selected.is_empty(),
            ..Self::default()
        }
    }

    pub fn sync(&mut self, selected: &[EntityId]) {
        if self.selected != selected {
            self.selected = selected.to_vec();
            self.image = None;
        }
        if selected.is_empty() {
            self.gathering = true;
            self.gathered = true;
        }
    }

    pub fn gathers(&self) -> bool {
        self.gathering
    }

    pub fn gathered(&self) -> bool {
        self.gathered
    }

    pub fn hover(
        &mut self,
        sketch: &Sketch,
        screen: &impl Screen,
        pointer: Option<Pointer>,
        faceting: Faceting,
    ) {
        if self.gathering {
            self.hover = None;
            self.highlight = None;
            self.image = None;
            return;
        }
        self.hover = pointer.and_then(|pointer| line_under(sketch, screen, pointer));
        self.highlight = self.highlight.filter(|line| is_mirror(sketch, *line));
        self.image = self
            .aim()
            .map(|line| (line, sketch.mirror_image(&self.selected, line, faceting)));
    }

    pub fn leave(&mut self) {
        self.hover = None;
        if self.highlight.is_none() {
            self.image = None;
        }
    }

    fn aim(&self) -> Option<EntityId> {
        self.highlight.or(self.hover)
    }

    pub fn preview(&self) -> Preview {
        let mut preview = Preview::default();
        if let Some((_, Ok(image))) = &self.image {
            preview.curves = image.curves.clone();
            preview.points = image.points.clone();
        }
        preview
    }

    pub fn highlighted_entities(&self) -> Vec<EntityId> {
        self.aim()
            .into_iter()
            .filter(|line| !line.is_reference())
            .collect()
    }

    pub fn label(&self, sketch: &Sketch) -> Option<String> {
        let (line, image) = self.image.as_ref()?;
        Some(match image {
            Ok(_) => format!(
                "Mirror {} about {}",
                count(self.selected.len(), "item", "items"),
                sketch.entity_label(*line)
            ),
            Err(error) => capitalized(&error.to_string()),
        })
    }

    pub fn prompt(&self) -> Prompt {
        match (self.gathering, self.selected.is_empty()) {
            (true, true) => Prompt {
                text: SELECT_FIRST,
                hint: Hint::Keys(GATHER_KEYS),
            },
            (true, false) => Prompt {
                text: SELECT_MORE,
                hint: Hint::Keys(GATHERED_KEYS),
            },
            (false, _) if self.gathered => Prompt {
                text: PROMPT,
                hint: Hint::Keys(LINE_KEYS),
            },
            (false, _) => Prompt {
                text: PROMPT,
                hint: Hint::Targets,
            },
        }
    }

    pub fn click(&mut self, model: &Model, feature: FeatureId) -> Outcome {
        match self.hover.filter(|_| !self.gathering) {
            Some(line) => self.mirror(model, feature, line),
            None => Outcome::Nothing,
        }
    }

    pub fn finish(&mut self, model: &Model, feature: FeatureId) -> Outcome {
        if !self.gathering {
            return self.activate(model, feature);
        }
        if self.selected.is_empty() {
            return Outcome::Refused(trimming::refusal(Tool::Mirror, NOTHING_SELECTED));
        }
        self.gathering = false;
        Outcome::Nothing
    }

    pub fn activate(&mut self, model: &Model, feature: FeatureId) -> Outcome {
        let aim = self.aim();
        self.highlight = None;
        match aim {
            Some(line) => self.mirror(model, feature, line),
            None => Outcome::Nothing,
        }
    }

    fn mirror(&mut self, model: &Model, feature: FeatureId, about: EntityId) -> Outcome {
        let selected = self.selected.clone();
        let outcome = trimming::reshaped(model, feature, TRANSACTION.to_owned(), |sketch| {
            sketch
                .mirror(&selected, about)
                .map(|_| ())
                .map_err(|error| trimming::refusal(Tool::Mirror, &error.to_string()))
        });
        if outcome.is_ok() {
            self.image = None;
        }
        outcome.into()
    }

    pub fn steppable(&self) -> Result<(), &'static str> {
        Ok(())
    }

    pub fn step(&mut self, sketch: &Sketch, step: isize) {
        let lines: Vec<EntityId> = [EntityId::HORIZONTAL_AXIS, EntityId::VERTICAL_AXIS]
            .into_iter()
            .chain(
                sketch
                    .entities()
                    .filter(|(_, entity)| matches!(entity, Entity::Line { .. }))
                    .map(|(id, _)| id),
            )
            .collect();
        let count = lines.len() as isize;
        let current = self
            .aim()
            .and_then(|aim| lines.iter().position(|line| *line == aim));
        let next = match current {
            Some(index) => (index as isize + step).rem_euclid(count),
            None if step < 0 => count - 1,
            None => 0,
        };
        self.highlight = usize::try_from(next)
            .ok()
            .and_then(|next| lines.get(next).copied());
    }

    pub fn highlight_needed(&self) -> Result<(), &'static str> {
        match self.highlight {
            Some(_) => Ok(()),
            None => Err(NO_LINE_HIGHLIGHTED),
        }
    }

    pub fn clear_highlight(&mut self) {
        self.highlight = None;
    }

    pub fn can_back_out(&self) -> bool {
        self.highlight.is_some() || (self.gathered && !self.gathering)
    }

    pub fn back_out(&mut self) {
        if self.highlight.take().is_none() {
            self.gathering = true;
            self.image = None;
        }
    }
}

fn is_mirror(sketch: &Sketch, line: EntityId) -> bool {
    line == EntityId::HORIZONTAL_AXIS
        || line == EntityId::VERTICAL_AXIS
        || matches!(sketch.entity(line), Some(Entity::Line { .. }))
}

fn line_under(sketch: &Sketch, screen: &impl Screen, pointer: Pointer) -> Option<EntityId> {
    let at = pointer.sketch;
    let axes = [
        (EntityId::HORIZONTAL_AXIS, Point2::new(at.x, 0.0)),
        (EntityId::VERTICAL_AXIS, Point2::new(0.0, at.y)),
    ];
    let lines = sketch
        .entities()
        .filter(|(_, entity)| matches!(entity, Entity::Line { .. }))
        .filter_map(|(line, _)| Some((line, sketch.closest_on_curve(line, at)?)));
    lines
        .chain(axes)
        .filter_map(|(line, on)| {
            let offset = screen.to_screen(on)?.distance(pointer.screen);
            (offset <= snap::CURVE_TOLERANCE).then_some((offset, line))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, line)| line)
}
