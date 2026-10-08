use caditor_geometry::{Point3, Rotation3};

pub const MAX_VIEW_NAME_CHARS: usize = 60;
pub const MAX_SAVED_VIEWS: usize = 64;
pub const HOME_VIEW_NAME: &str = "Isometric";
const NUMBERED_NAME: &str = "View";
const UNIT_TOLERANCE: f64 = 1e-12;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SavedView {
    pub target: Point3,
    pub orientation: Rotation3,
    pub distance: f64,
}

impl SavedView {
    pub fn is_usable(&self) -> bool {
        self.target.is_finite()
            && self.orientation.is_finite()
            && self.orientation.length_squared() > 0.0
            && self.distance.is_finite()
            && self.distance > 0.0
    }

    #[must_use]
    pub fn normalized(self) -> Self {
        let off_unit = (self.orientation.length() - 1.0).abs() > UNIT_TOLERANCE;
        match self.is_usable() && off_unit {
            true => Self {
                orientation: self.orientation.normalize(),
                ..self
            },
            false => self,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NamedView {
    pub name: String,
    pub view: SavedView,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SavedViews {
    pub named: Vec<NamedView>,
    pub home: Option<SavedView>,
}

pub fn view_name(text: &str) -> String {
    text.trim()
        .split(['\r', '\n'])
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn same_name(left: &str, right: &str) -> bool {
    left.to_lowercase() == right.to_lowercase()
}

impl SavedViews {
    pub fn is_empty(&self) -> bool {
        self.named.is_empty() && self.home.is_none()
    }

    pub fn position(&self, name: &str) -> Option<usize> {
        self.named
            .iter()
            .position(|named| same_name(&named.name, name))
    }

    pub fn is_taken(&self, name: &str) -> bool {
        self.position(name).is_some()
    }

    pub fn unused_name(&self) -> String {
        (self.named.len() + 1..)
            .map(|number| format!("{NUMBERED_NAME} {number}"))
            .find(|name| !self.is_taken(name))
            .unwrap_or_else(|| NUMBERED_NAME.to_owned())
    }

    #[must_use]
    pub fn normalized(self) -> Self {
        Self {
            named: self
                .named
                .into_iter()
                .map(|named| NamedView {
                    name: view_name(&named.name),
                    view: named.view.normalized(),
                })
                .collect(),
            home: self.home.map(SavedView::normalized),
        }
    }

    pub fn unusable(&self) -> Option<&str> {
        self.named
            .iter()
            .find(|named| !named.view.is_usable())
            .map(|named| named.name.as_str())
            .or_else(|| {
                self.home
                    .filter(|view| !view.is_usable())
                    .map(|_| HOME_VIEW_NAME)
            })
    }

    pub fn heap_size(&self) -> usize {
        self.named
            .iter()
            .map(|named| size_of::<NamedView>() + named.name.len())
            .sum()
    }
}
