use std::collections::BTreeSet;

use caditor_expression::{Expression, ParameterId};
use caditor_geometry::Vector3;
use caditor_kernel::MAX_SIZE;

use crate::{
    document::{Document, FeatureId, ParameterUser},
    edit::{Edit, EditError, Transaction},
    views::view_name,
};

pub const MAX_DRAWINGS: usize = 32;
pub const MAX_DRAWING_NAME_CHARS: usize = 60;
pub const MAX_DRAWING_BODIES: usize = 4096;
pub const MAX_SCALE_TERM: u32 = 1000;
pub const SECTION_LETTERS: [char; 23] = [
    'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'J', 'K', 'L', 'M', 'N', 'P', 'R', 'S', 'T', 'U', 'V',
    'W', 'X', 'Y', 'Z',
];
pub const MAX_SECTION_VIEWS: usize = SECTION_LETTERS.len();
pub const MAX_SECTION_OFFSET: f64 = MAX_SIZE;
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

impl StandardView {
    pub const SECTIONED: [Self; 4] = [Self::Front, Self::Top, Self::Right, Self::Left];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SectionPlane {
    Yz,
    Xz,
    Xy,
}

impl SectionPlane {
    pub const ALL: [Self; 3] = [Self::Yz, Self::Xz, Self::Xy];

    pub fn name(self) -> &'static str {
        match self {
            Self::Yz => "YZ plane",
            Self::Xz => "XZ plane",
            Self::Xy => "XY plane",
        }
    }

    pub fn axis_name(self) -> &'static str {
        match self {
            Self::Yz => "X",
            Self::Xz => "Y",
            Self::Xy => "Z",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Yz => "yz",
            Self::Xz => "xz",
            Self::Xy => "xy",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|plane| plane.key() == key)
    }

    pub fn normal(self) -> Vector3 {
        match self {
            Self::Yz => Vector3::X,
            Self::Xz => Vector3::Y,
            Self::Xy => Vector3::Z,
        }
    }

    pub fn is_edge_on_in(self, view: StandardView) -> bool {
        match view {
            StandardView::Front => self != Self::Xz,
            StandardView::Top => self != Self::Xy,
            StandardView::Right | StandardView::Left => self != Self::Yz,
            StandardView::Isometric => false,
        }
    }

    pub fn edge_on_in(view: StandardView) -> impl Iterator<Item = Self> {
        Self::ALL
            .into_iter()
            .filter(move |plane| plane.is_edge_on_in(view))
    }

    fn look_and_up(self) -> (Vector3, Vector3) {
        match self {
            Self::Yz => (-Vector3::X, Vector3::Z),
            Self::Xz => (Vector3::Y, Vector3::Z),
            Self::Xy => (-Vector3::Z, Vector3::Y),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum SectionPlace {
    #[default]
    Centre,
    Offset(Expression),
}

impl SectionPlace {
    pub fn offset(millimetres: f64) -> Self {
        Self::Offset(Expression::number(millimetres))
    }

    pub fn heap_size(&self) -> usize {
        match self {
            Self::Centre => 0,
            Self::Offset(offset) => offset.heap_size(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SectionView {
    pub letter: char,
    pub parent: StandardView,
    pub plane: SectionPlane,
    pub place: SectionPlace,
    pub reversed: bool,
}

impl SectionView {
    pub fn label(&self) -> String {
        format!("{0}\u{2013}{0}", self.letter)
    }

    pub fn look_and_up(&self) -> (Vector3, Vector3) {
        let (look, up) = self.plane.look_and_up();
        if self.reversed {
            (-look, up)
        } else {
            (look, up)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DrawnView {
    Standard(StandardView),
    Section(char),
}

impl DrawnView {
    pub fn describe(self) -> String {
        match self {
            Self::Standard(view) => format!("{} view", view.name().to_lowercase()),
            Self::Section(letter) => format!("section view {letter}\u{2013}{letter}"),
        }
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

#[derive(Debug, Clone, PartialEq)]
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
    pub sections: Vec<SectionView>,
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
            sections: Vec::new(),
        }
    }

    pub fn heap_size(&self) -> usize {
        self.name.len()
            + self.views.len() * size_of::<StandardView>()
            + size_of_val(self.bodies.as_slice())
            + size_of_val(self.sections.as_slice())
            + self
                .sections
                .iter()
                .map(|section| section.place.heap_size())
                .sum::<usize>()
    }

    pub fn section(&self, letter: char) -> Option<&SectionView> {
        self.sections
            .iter()
            .find(|section| section.letter == letter)
    }

    pub fn section_mut(&mut self, letter: char) -> Option<&mut SectionView> {
        self.sections
            .iter_mut()
            .find(|section| section.letter == letter)
    }

    pub fn unused_section_letter(&self) -> Option<char> {
        SECTION_LETTERS
            .into_iter()
            .find(|letter| self.section(*letter).is_none())
    }

    pub fn new_section(&self) -> Option<SectionView> {
        let letter = self.unused_section_letter()?;
        let parent = StandardView::SECTIONED
            .into_iter()
            .find(|view| self.views.contains(view))
            .unwrap_or(StandardView::Front);
        let plane = SectionPlane::edge_on_in(parent).next()?;
        Some(SectionView {
            letter,
            parent,
            plane,
            place: SectionPlace::Centre,
            reversed: false,
        })
    }

    fn checked_letters(&self) -> Result<(), EditError> {
        if self.sections.len() > MAX_SECTION_VIEWS {
            return Err(EditError::DrawingTooManySections {
                name: self.name.clone(),
                sections: self.sections.len(),
            });
        }
        let letters = self.sections.iter().map(|section| section.letter);
        for (index, letter) in letters.clone().enumerate() {
            if !SECTION_LETTERS.contains(&letter) {
                return Err(EditError::SectionLetterUnusable(letter));
            }
            if letters.clone().take(index).any(|other| other == letter) {
                return Err(EditError::SectionLetterRepeated(letter));
            }
        }
        Ok(())
    }

    fn checked_sections(&self, document: &Document) -> Result<(), EditError> {
        for section in &self.sections {
            let letter = section.letter;
            if !section.plane.is_edge_on_in(section.parent) {
                return Err(EditError::SectionPlaneFacesParent {
                    letter,
                    plane: section.plane.name(),
                    view: section.parent.name(),
                });
            }
            if let SectionPlace::Offset(offset) = &section.place
                && offset
                    .parameters()
                    .into_iter()
                    .any(|id| document.parameter(id).is_none())
            {
                return Err(EditError::MissingParameter);
            }
        }
        Ok(())
    }

    fn parameters(&self) -> impl Iterator<Item = ParameterId> + '_ {
        self.sections
            .iter()
            .flat_map(|section| match &section.place {
                SectionPlace::Centre => Vec::new(),
                SectionPlace::Offset(offset) => offset.parameters().into_iter().collect(),
            })
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
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

    pub fn parameters(&self) -> impl Iterator<Item = ParameterId> + '_ {
        self.drawings.iter().flat_map(Drawing::parameters)
    }

    pub(crate) fn checked(self, document: &Document) -> Result<Self, EditError> {
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
            drawing.checked_letters()?;
            drawing.checked_sections(document)?;
        }
        Ok(drawings)
    }
}

impl Document {
    pub(crate) fn drawing_user_ids(&self, parameter: ParameterId) -> Vec<ParameterUser> {
        self.drawings()
            .drawings
            .iter()
            .filter(|drawing| drawing.parameters().any(|used| used == parameter))
            .map(|drawing| ParameterUser::Drawing(drawing.id))
            .collect()
    }

    pub(crate) fn drawing_users(&self, parameter: ParameterId) -> Vec<String> {
        self.drawing_user_ids(parameter)
            .into_iter()
            .filter_map(|user| self.drawing_user_name(user))
            .collect()
    }

    pub fn drawing_user_name(&self, user: ParameterUser) -> Option<String> {
        match user {
            ParameterUser::Drawing(id) => self.drawing(id).map(|drawing| drawing.name.clone()),
            ParameterUser::Parameter(_)
            | ParameterUser::Feature(_)
            | ParameterUser::Configuration(_)
            | ParameterUser::Configurations => None,
        }
    }

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

    pub fn adding_section_view(&self, id: DrawingId) -> Result<(Transaction, char), EditError> {
        let mut drawings = self.drawings().clone();
        let drawing = drawings.get_mut(id).ok_or(EditError::MissingDrawing)?;
        let section = drawing
            .new_section()
            .ok_or_else(|| EditError::DrawingTooManySections {
                name: drawing.name.clone(),
                sections: drawing.sections.len().saturating_add(1),
            })?;
        let letter = section.letter;
        drawing.sections.push(section);
        let transaction = self.drawings_changed("New section view", drawings)?;
        Ok((transaction, letter))
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
