use caditor_document::{FeatureId, Transaction};
use caditor_expression::Dimension;
use caditor_geometry::Point2;
use caditor_sketch::{
    Entity, EntityId, Faceting, GearCentre, GearOutline, MAX_TEETH, MIN_TEETH, Sketch, SpurGear,
};

use crate::{
    drawing::Preview,
    editing::Tool,
    field::Expected,
    model::Model,
    modifying::{Hint, Outcome, Prompt, length_text},
    patterning,
    snap::{self, Pointer, Screen},
    trimming::{self, capitalized},
    units::LengthUnit,
};

pub const TRANSACTION: &str = "Draw spur gear";
pub const PROMPT: &str =
    "Click where the gear's centre goes: on a point, or anywhere in the sketch";
const KEYS: &str = "Enter: draw it at the selected point or the origin   Esc: back to Select";
const NOTHING_HIGHLIGHTED: &str =
    "Highlight a point first, with Highlight the next item in the view";
const WHOLE_NUMBER_TOLERANCE: f64 = 1e-9;
const LENGTH: Expected = Expected {
    dimension: Some(Dimension::LENGTH),
    non_negative: false,
};
const ANGLE: Expected = Expected {
    dimension: Some(Dimension::ANGLE),
    non_negative: false,
};
const PLAIN: Expected = Expected {
    dimension: Some(Dimension::NONE),
    non_negative: false,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GearField {
    Module,
    Teeth,
    PressureAngle,
    ProfileShift,
    RootFillet,
    Bore,
}

impl GearField {
    pub const ALL: [Self; 6] = [
        Self::Module,
        Self::Teeth,
        Self::PressureAngle,
        Self::ProfileShift,
        Self::RootFillet,
        Self::Bore,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Module => "Module",
            Self::Teeth => "Teeth",
            Self::PressureAngle => "Pressure angle",
            Self::ProfileShift => "Profile shift",
            Self::RootFillet => "Root fillet",
            Self::Bore => "Bore",
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            Self::Module => "The pitch diameter over the number of teeth; meshing gears share it",
            Self::Teeth => "A whole number of teeth",
            Self::PressureAngle => "The angle the teeth push along, usually 20°",
            Self::ProfileShift => {
                "How far the teeth move out, in modules: positive to avoid undercut on few teeth"
            }
            Self::RootFillet => "The radius rounding the root of each gap; 0 for a sharp root",
            Self::Bore => "The diameter of the hole in the middle; empty or 0 for none",
        }
    }

    fn initial(self) -> &'static str {
        match self {
            Self::Module => "2 mm",
            Self::Teeth => "20",
            Self::PressureAngle => "20°",
            Self::ProfileShift => "0",
            Self::RootFillet => "0.75 mm",
            Self::Bore => "",
        }
    }

    fn expected(self) -> Expected {
        match self {
            Self::Module | Self::RootFillet | Self::Bore => LENGTH,
            Self::PressureAngle => ANGLE,
            Self::Teeth | Self::ProfileShift => PLAIN,
        }
    }

    fn may_be_empty(self) -> bool {
        matches!(self, Self::Bore)
    }

    fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|field| *field == self)
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GearSettings {
    texts: [String; 6],
    evaluated: Option<Result<SpurGear, String>>,
}

impl Default for GearSettings {
    fn default() -> Self {
        Self {
            texts: GearField::ALL.map(|field| field.initial().to_owned()),
            evaluated: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Evaluation {
    pub problems: Vec<(GearField, String)>,
    pub gear: Result<SpurGear, String>,
}

impl GearSettings {
    pub fn text(&self, field: GearField) -> &str {
        self.texts.get(field.index()).map_or("", String::as_str)
    }

    pub fn text_mut(&mut self, field: GearField) -> Option<&mut String> {
        self.texts.get_mut(field.index())
    }

    pub fn gear(&self) -> Option<&Result<SpurGear, String>> {
        self.evaluated.as_ref()
    }

    pub fn evaluate(&mut self, model: &Model) -> Evaluation {
        let mut problems = Vec::new();
        let mut value = |field: GearField| -> f64 {
            match value_of(model, field, self.text(field)) {
                Ok(value) => value,
                Err(problem) => {
                    problems.push((field, problem));
                    0.0
                }
            }
        };
        let module = value(GearField::Module);
        let teeth = value(GearField::Teeth);
        let pressure_angle = value(GearField::PressureAngle);
        let profile_shift = value(GearField::ProfileShift);
        let root_fillet = value(GearField::RootFillet);
        let bore = value(GearField::Bore);
        let gear = match problems.first() {
            Some((field, _)) => Err(format!("Mend the {} first", field.label().to_lowercase())),
            None => {
                let gear = SpurGear {
                    module,
                    teeth: teeth as u32,
                    pressure_angle,
                    profile_shift,
                    root_fillet,
                    bore,
                };
                gear.outline(Point2::ZERO)
                    .map(|_| gear)
                    .map_err(|error| capitalized(&error.to_string()))
            }
        };
        self.evaluated = Some(gear.clone());
        Evaluation { problems, gear }
    }
}

fn value_of(model: &Model, field: GearField, text: &str) -> Result<f64, String> {
    if field.may_be_empty() && text.trim().is_empty() {
        return Ok(0.0);
    }
    let label = field.label().to_lowercase();
    let value = patterning::quantity(model, text, field.expected(), &label)?.value;
    if field != GearField::Teeth {
        return Ok(value);
    }
    let rounded = value.round();
    if (value - rounded).abs() > WHOLE_NUMBER_TOLERANCE
        || !(f64::from(MIN_TEETH)..=f64::from(MAX_TEETH)).contains(&rounded)
    {
        return Err(format!(
            "teeth: Use a whole number from {MIN_TEETH} to {MAX_TEETH}"
        ));
    }
    Ok(rounded)
}

pub fn lone_point(sketch: &Sketch, selected: &[EntityId]) -> Option<EntityId> {
    match selected {
        [only] if matches!(sketch.entity(*only), Some(Entity::Point(_))) => Some(*only),
        _ => None,
    }
}

pub fn centre_words(sketch: &Sketch, centre: Option<EntityId>) -> String {
    match centre {
        Some(point) => sketch.entity_label(point),
        None => "the origin".to_owned(),
    }
}

pub fn draw(
    model: &Model,
    feature: FeatureId,
    gear: &SpurGear,
    centre: GearCentre,
) -> Result<Transaction, String> {
    trimming::reshaped(model, feature, TRANSACTION.to_owned(), |sketch| {
        sketch
            .add_gear(gear, centre)
            .map(|_| ())
            .map_err(|error| trimming::refusal(Tool::Gear, &error.to_string()))
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Aim {
    Point(EntityId, Point2),
    Free(Point2),
}

impl Aim {
    fn at(self) -> Point2 {
        match self {
            Self::Point(_, at) | Self::Free(at) => at,
        }
    }

    fn centre(self) -> GearCentre {
        match self {
            Self::Point(point, _) => GearCentre::Point(point),
            Self::Free(at) => GearCentre::Free(at),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Gearing {
    selected: Option<EntityId>,
    hover: Option<Aim>,
    highlight: Option<EntityId>,
    shown: Option<(Aim, SpurGear, Result<GearOutline, String>)>,
}

impl Gearing {
    pub fn starting(sketch: &Sketch, selected: &[EntityId]) -> Self {
        Self {
            selected: lone_point(sketch, selected),
            ..Self::default()
        }
    }

    pub fn sync(&mut self, sketch: &Sketch) {
        let exists = |point: &EntityId| is_centre(sketch, *point);
        self.selected = self.selected.filter(exists);
        self.highlight = self.highlight.filter(exists);
    }

    pub fn hover(
        &mut self,
        sketch: &Sketch,
        screen: &impl Screen,
        pointer: Option<Pointer>,
        gear: Option<&Result<SpurGear, String>>,
    ) {
        self.hover = pointer.map(|pointer| match point_under(sketch, screen, pointer) {
            Some(point) => Aim::Point(point, sketch.point(point).unwrap_or(pointer.sketch)),
            None => Aim::Free(pointer.sketch),
        });
        self.refresh(sketch, gear);
    }

    pub fn leave(&mut self) {
        self.hover = None;
        if self.highlight.is_none() {
            self.shown = None;
        }
    }

    fn aim(&self, sketch: &Sketch) -> Option<Aim> {
        self.highlight
            .and_then(|point| Some(Aim::Point(point, sketch.point(point)?)))
            .or(self.hover)
    }

    fn refresh(&mut self, sketch: &Sketch, gear: Option<&Result<SpurGear, String>>) {
        let (Some(aim), Some(Ok(gear))) = (self.aim(sketch), gear) else {
            self.shown = None;
            return;
        };
        let fresh =
            matches!(&self.shown, Some((shown, drawn, _)) if *shown == aim && drawn == gear);
        if !fresh {
            let outline = gear
                .outline(aim.at())
                .map_err(|error| capitalized(&error.to_string()));
            self.shown = Some((aim, *gear, outline));
        }
    }

    pub fn preview(&self, faceting: Faceting) -> Preview {
        let mut preview = Preview::default();
        if let Some((aim, _, Ok(outline))) = &self.shown {
            preview.curves = outline.faceted(faceting);
            preview.guides = outline
                .construction(faceting)
                .iter()
                .flat_map(|circle| {
                    circle
                        .windows(2)
                        .filter_map(|pair| match pair {
                            [from, to] => Some([*from, *to]),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                })
                .collect();
            preview.points = vec![aim.at()];
        }
        preview
    }

    pub fn highlighted_entities(&self) -> Vec<EntityId> {
        let aimed = match self.shown {
            Some((Aim::Point(point, _), ..)) => Some(point),
            _ => self.highlight,
        };
        aimed
            .or(self.selected)
            .filter(|point| !point.is_reference())
            .into_iter()
            .collect()
    }

    pub fn label(
        &self,
        sketch: &Sketch,
        unit: LengthUnit,
        gear: Option<&Result<SpurGear, String>>,
    ) -> Option<String> {
        if let Some(Err(problem)) = gear {
            return Some(problem.clone());
        }
        let (aim, gear, outline) = self.shown.as_ref()?;
        Some(match outline {
            Ok(outline) => {
                let place = match aim {
                    Aim::Point(point, _) => format!(" centred on {}", sketch.entity_label(*point)),
                    Aim::Free(_) => String::new(),
                };
                format!(
                    "Spur gear of {} teeth, module {}, {} across the tips{place}",
                    gear.teeth,
                    length_text(unit, gear.module),
                    length_text(unit, 2.0 * outline.circles.tip)
                )
            }
            Err(problem) => problem.clone(),
        })
    }

    pub fn prompt(&self) -> Prompt {
        Prompt {
            text: PROMPT,
            hint: Hint::Keys(KEYS),
        }
    }

    pub fn click(
        &mut self,
        model: &Model,
        feature: FeatureId,
        gear: Option<&Result<SpurGear, String>>,
    ) -> Outcome {
        match self.hover {
            Some(aim) => self.draw_at(model, feature, gear, aim.centre()),
            None => Outcome::Nothing,
        }
    }

    pub fn finish(
        &mut self,
        model: &Model,
        feature: FeatureId,
        gear: Option<&Result<SpurGear, String>>,
    ) -> Outcome {
        let centre = self.highlight.or(self.selected).unwrap_or(EntityId::ORIGIN);
        self.highlight = None;
        self.draw_at(model, feature, gear, GearCentre::Point(centre))
    }

    pub fn activate(
        &mut self,
        model: &Model,
        feature: FeatureId,
        gear: Option<&Result<SpurGear, String>>,
    ) -> Outcome {
        match self.highlight.take() {
            Some(point) => self.draw_at(model, feature, gear, GearCentre::Point(point)),
            None => Outcome::Nothing,
        }
    }

    fn draw_at(
        &mut self,
        model: &Model,
        feature: FeatureId,
        gear: Option<&Result<SpurGear, String>>,
        centre: GearCentre,
    ) -> Outcome {
        let drawn = match gear {
            Some(Ok(gear)) => draw(model, feature, gear, centre),
            Some(Err(problem)) => Err(trimming::refusal(Tool::Gear, problem)),
            None => Err(trimming::refusal(Tool::Gear, "its values are not read yet")),
        };
        if drawn.is_ok() {
            self.shown = None;
        }
        drawn.into()
    }

    pub fn steppable(&self) -> Result<(), &'static str> {
        Ok(())
    }

    pub fn step(&mut self, sketch: &Sketch, step: isize) {
        let points: Vec<EntityId> = std::iter::once(EntityId::ORIGIN)
            .chain(
                sketch
                    .entities()
                    .filter(|(_, entity)| matches!(entity, Entity::Point(_)))
                    .map(|(id, _)| id),
            )
            .collect();
        let total = points.len() as isize;
        let current = self
            .highlight
            .and_then(|aim| points.iter().position(|point| *point == aim));
        let next = match current {
            Some(index) => (index as isize + step).rem_euclid(total),
            None if step < 0 => total - 1,
            None => 0,
        };
        self.highlight = usize::try_from(next)
            .ok()
            .and_then(|next| points.get(next).copied());
    }

    pub fn highlight_needed(&self) -> Result<(), &'static str> {
        match self.highlight {
            Some(_) => Ok(()),
            None => Err(NOTHING_HIGHLIGHTED),
        }
    }

    pub fn clear_highlight(&mut self) {
        self.highlight = None;
    }

    pub fn can_back_out(&self) -> bool {
        self.highlight.is_some()
    }

    pub fn back_out(&mut self) {
        self.highlight = None;
    }
}

fn is_centre(sketch: &Sketch, point: EntityId) -> bool {
    point == EntityId::ORIGIN || matches!(sketch.entity(point), Some(Entity::Point(_)))
}

fn point_under(sketch: &Sketch, screen: &impl Screen, pointer: Pointer) -> Option<EntityId> {
    sketch
        .entities()
        .filter_map(|(id, entity)| match entity {
            Entity::Point(position) => Some((id, *position)),
            Entity::Line { .. }
            | Entity::Circle { .. }
            | Entity::Arc { .. }
            | Entity::Spline { .. }
            | Entity::Ellipse { .. }
            | Entity::EllipticalArc { .. } => None,
        })
        .chain([(EntityId::ORIGIN, Point2::ZERO)])
        .filter_map(|(id, position)| {
            let offset = screen.to_screen(position)?.distance(pointer.screen);
            (offset <= snap::POINT_TOLERANCE).then_some((offset, id))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, id)| id)
}
