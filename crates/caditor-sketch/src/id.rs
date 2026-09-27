use std::fmt;

pub(crate) const FIRST_UNSTORABLE_ID: u64 = 1 << 63;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityId(u64);

impl EntityId {
    pub const ORIGIN: Self = Self(u64::MAX);
    pub const HORIZONTAL_AXIS: Self = Self(u64::MAX - 1);
    pub const VERTICAL_AXIS: Self = Self(u64::MAX - 2);
    pub const REFERENCES: [Self; 3] = [Self::ORIGIN, Self::HORIZONTAL_AXIS, Self::VERTICAL_AXIS];

    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u64 {
        self.0
    }

    pub fn is_reference(self) -> bool {
        self.reference().is_some()
    }

    pub fn reference(self) -> Option<Reference> {
        Reference::ALL
            .into_iter()
            .find(|reference| reference.id() == self)
    }
}

impl fmt::Display for EntityId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConstraintId(u64);

impl ConstraintId {
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

impl fmt::Display for ConstraintId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Reference {
    Origin,
    HorizontalAxis,
    VerticalAxis,
}

impl Reference {
    pub const ALL: [Self; 3] = [Self::Origin, Self::HorizontalAxis, Self::VerticalAxis];

    pub fn id(self) -> EntityId {
        match self {
            Self::Origin => EntityId::ORIGIN,
            Self::HorizontalAxis => EntityId::HORIZONTAL_AXIS,
            Self::VerticalAxis => EntityId::VERTICAL_AXIS,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Origin => "Origin",
            Self::HorizontalAxis => "Horizontal axis",
            Self::VerticalAxis => "Vertical axis",
        }
    }
}
