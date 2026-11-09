use std::collections::BTreeSet;

use crate::{
    document::{Document, FeatureId},
    edit::{Edit, EditError, Transaction},
    views::view_name,
};

pub const MAX_DRAWINGS: usize = 32;
pub const MAX_DRAWING_NAME_CHARS: usize = 60;
pub const MAX_DRAWING_BODIES: usize = 4096;
pub const MAX_SCALE_TERM: u32 = 1000;
const NUMBERED_NAME: &str = "Drawing";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DrawingId(u64);

impl DrawingId {
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum SheetSize {
    A4,
    #[default]
    A3,
    A2,
    A1,
    A0,
}

impl SheetSize {
    pub const ALL: [Self; 5] = [Self::A4, Self::A3, Self::A2, Self::A1, Self::A0];

    pub fn name(self) -> &'static str {
        match self {
            Self::A4 => "A4",
            Self::A3 => "A3",
            Self::A2 => "A2",
            Self::A1 => "A1",
            Self::A0 => "A0",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::A4 => "a4",
            Self::A3 => "a3",
            Self::A2 => "a2",
            Self::A1 => "a1",
            Self::A0 => "a0",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|size| size.key() == key)
    }

    pub fn short_and_long_sides(self) -> (f64, f64) {
        match self {
            Self::A4 => (210.0, 297.0),
            Self::A3 => (297.0, 420.0),
            Self::A2 => (420.0, 594.0),
            Self::A1 => (594.0, 841.0),
            Self::A0 => (841.0, 1189.0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum SheetOrientation {
    #[default]
    Landscape,
    Portrait,
}

impl SheetOrientation {
    pub const ALL: [Self; 2] = [Self::Landscape, Self::Portrait];

    pub fn name(self) -> &'static str {
        match self {
            Self::Landscape => "Landscape",
            Self::Portrait => "Portrait",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Landscape => "landscape",
            Self::Portrait => "portrait",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|orientation| orientation.key() == key)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum Projection {
    #[default]
    FirstAngle,
    ThirdAngle,
}

impl Projection {
    pub const ALL: [Self; 2] = [Self::FirstAngle, Self::ThirdAngle];

    pub fn name(self) -> &'static str {
        match self {
            Self::FirstAngle => "First angle",
            Self::ThirdAngle => "Third angle",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::FirstAngle => "first_angle",
            Self::ThirdAngle => "third_angle",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|projection| projection.key() == key)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StandardView {
    Front,
    Top,
    Right,
    Left,
    Isometric,
}

impl StandardView {
    pub const ALL: [Self; 5] = [
        Self::Front,
        Self::Top,
        Self::Right,
        Self::Left,
        Self::Isometric,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Front => "Front",
            Self::Top => "Top",
            Self::Right => "Right",
            Self::Left => "Left",
            Self::Isometric => "Isometric",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Front => "front",
            Self::Top => "top",
            Self::Right => "right",
            Self::Left => "left",
            Self::Isometric => "isometric",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|view| view.key() == key)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScaleRatio {
    pub sheet: u32,
    pub model: u32,
}

impl ScaleRatio {
    pub const PREFERRED: [Self; 15] = [
        Self::new(50, 1),
        Self::new(20, 1),
        Self::new(10, 1),
        Self::new(5, 1),
        Self::new(2, 1),
        Self::new(1, 1),
        Self::new(1, 2),
        Self::new(1, 5),
        Self::new(1, 10),
        Self::new(1, 20),
        Self::new(1, 50),
        Self::new(1, 100),
        Self::new(1, 200),
        Self::new(1, 500),
        Self::new(1, 1000),
    ];

    pub const fn new(sheet: u32, model: u32) -> Self {
        Self { sheet, model }
    }

    pub fn factor(self) -> f64 {
        f64::from(self.sheet) / f64::from(self.model.max(1))
    }

    pub fn label(self) -> String {
        format!("{}:{}", self.sheet, self.model)
    }

    pub fn is_usable(self) -> bool {
        (1..=MAX_SCALE_TERM).contains(&self.sheet) && (1..=MAX_SCALE_TERM).contains(&self.model)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum DrawingScale {
    #[default]
    Fit,
    Ratio(ScaleRatio),
}

impl DrawingScale {
    pub fn label(self) -> String {
        match self {
            Self::Fit => "Fit to the sheet".to_owned(),
            Self::Ratio(ratio) => ratio.label(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drawing {
    pub id: DrawingId,
    pub name: String,
    pub size: SheetSize,
    pub orientation: SheetOrientation,
    pub projection: Projection,
    pub scale: DrawingScale,
    pub views: BTreeSet<StandardView>,
    pub bodies: Vec<FeatureId>,
    pub hidden_lines: bool,
}

impl Drawing {
    pub fn new(id: DrawingId, name: String) -> Self {
        Self {
            id,
            name,
            size: SheetSize::default(),
            orientation: SheetOrientation::default(),
            projection: Projection::default(),
            scale: DrawingScale::default(),
            views: [
                StandardView::Front,
                StandardView::Top,
                StandardView::Right,
                StandardView::Isometric,
            ]
            .into_iter()
            .collect(),
            bodies: Vec::new(),
            hidden_lines: true,
        }
    }

    pub fn heap_size(&self) -> usize {
        self.name.len()
            + self.views.len() * size_of::<StandardView>()
            + size_of_val(self.bodies.as_slice())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Drawings {
    pub drawings: Vec<Drawing>,
    pub next_id: u64,
}

fn same_name(left: &str, right: &str) -> bool {
    left.to_lowercase() == right.to_lowercase()
}

pub fn drawing_name(text: &str) -> String {
    view_name(text)
}

impl Drawings {
    pub fn is_empty(&self) -> bool {
        self.drawings.is_empty()
    }

    pub fn get(&self, id: DrawingId) -> Option<&Drawing> {
        self.drawings.iter().find(|drawing| drawing.id == id)
    }

    pub fn get_mut(&mut self, id: DrawingId) -> Option<&mut Drawing> {
        self.drawings.iter_mut().find(|drawing| drawing.id == id)
    }

    pub fn is_taken(&self, name: &str, besides: Option<DrawingId>) -> bool {
        self.drawings
            .iter()
            .any(|drawing| Some(drawing.id) != besides && same_name(&drawing.name, name))
    }

    pub fn unused_name(&self) -> String {
        (self.drawings.len() + 1..)
            .map(|number| format!("{NUMBERED_NAME} {number}"))
            .find(|name| !self.is_taken(name, None))
            .unwrap_or_else(|| NUMBERED_NAME.to_owned())
    }

    pub fn first_free_id(&self) -> u64 {
        self.drawings
            .iter()
            .map(|drawing| drawing.id.raw().saturating_add(1))
            .max()
            .unwrap_or(0)
            .max(self.next_id)
    }

    pub fn bodies(&self) -> impl Iterator<Item = FeatureId> + '_ {
        self.drawings
            .iter()
            .flat_map(|drawing| drawing.bodies.iter().copied())
    }

    pub fn same_content(&self, other: &Self) -> bool {
        self.drawings == other.drawings
    }

    #[must_use]
    pub fn normalized(self) -> Self {
        Self {
            drawings: self
                .drawings
                .into_iter()
                .map(|drawing| Drawing {
                    name: drawing_name(&drawing.name),
                    ..drawing
                })
                .collect(),
            next_id: self.next_id,
        }
    }

    pub fn heap_size(&self) -> usize {
        self.drawings
            .iter()
            .map(|drawing| size_of::<Drawing>() + drawing.heap_size())
            .sum()
    }

    pub(crate) fn checked(self) -> Result<Self, EditError> {
        let drawings = self.normalized();
        if drawings.drawings.len() > MAX_DRAWINGS {
            return Err(EditError::TooManyDrawings);
        }
        for (index, drawing) in drawings.drawings.iter().enumerate() {
            let length = drawing.name.chars().count();
            if length == 0 {
                return Err(EditError::DrawingNameEmpty);
            }
            if length > MAX_DRAWING_NAME_CHARS {
                return Err(EditError::DrawingNameTooLong { length });
            }
            let earlier = drawings.drawings.iter().take(index);
            if earlier
                .clone()
                .any(|other| same_name(&other.name, &drawing.name))
            {
                return Err(EditError::DrawingNameTaken(drawing.name.clone()));
            }
            if earlier.clone().any(|other| other.id == drawing.id) {
                return Err(EditError::DrawingIdRepeated(drawing.id.raw()));
            }
            if drawing.bodies.len() > MAX_DRAWING_BODIES {
                return Err(EditError::DrawingTooManyBodies {
                    name: drawing.name.clone(),
                    bodies: drawing.bodies.len(),
                });
            }
            if let DrawingScale::Ratio(ratio) = drawing.scale
                && !ratio.is_usable()
            {
                return Err(EditError::DrawingScaleOutOfRange {
                    sheet: ratio.sheet,
                    model: ratio.model,
                });
            }
        }
        Ok(drawings)
    }
}

impl Document {
    pub fn drawings(&self) -> &Drawings {
        &self.drawings
    }

    pub fn drawing(&self, id: DrawingId) -> Option<&Drawing> {
        self.drawings.get(id)
    }

    fn drawings_changed(&self, label: &str, drawings: Drawings) -> Result<Transaction, EditError> {
        let transaction = Transaction::single(
            label,
            Edit::SetDrawings {
                drawings: Box::new(drawings),
            },
        );
        self.check(&transaction)?;
        Ok(transaction)
    }

    pub fn adding_drawing(
        &self,
        bodies: Vec<FeatureId>,
    ) -> Result<(Transaction, DrawingId), EditError> {
        let mut drawings = self.drawings().clone();
        let id = DrawingId::from_raw(drawings.first_free_id());
        let mut drawing = Drawing::new(id, drawings.unused_name());
        drawing.bodies = bodies;
        drawings.drawings.push(drawing);
        drawings.next_id = id.raw().saturating_add(1);
        let transaction = self.drawings_changed("New drawing", drawings)?;
        Ok((transaction, id))
    }

    pub fn changing_drawing(
        &self,
        id: DrawingId,
        label: &str,
        change: impl FnOnce(&mut Drawing),
    ) -> Result<Transaction, EditError> {
        let mut drawings = self.drawings().clone();
        let drawing = drawings.get_mut(id).ok_or(EditError::MissingDrawing)?;
        change(drawing);
        self.drawings_changed(label, drawings)
    }

    pub fn deleting_drawing(&self, id: DrawingId) -> Result<Transaction, EditError> {
        let mut drawings = self.drawings().clone();
        let before = drawings.drawings.len();
        drawings.drawings.retain(|drawing| drawing.id != id);
        if drawings.drawings.len() == before {
            return Err(EditError::MissingDrawing);
        }
        self.drawings_changed("Delete drawing", drawings)
    }
}
