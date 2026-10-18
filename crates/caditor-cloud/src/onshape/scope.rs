use crate::{
    catalogue::{DesignId, FolderId},
    encoding::percent_encode,
};

const MINE: &str = "documents";
const SHARED: &str = "shared";
const RECENT: &str = "recent";
const PUBLIC: &str = "public";
const COMPANY: &str = "company";
const TEAM: &str = "team";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Mine,
    Shared,
    Recent,
    Public,
    Company(String),
    Team(String),
}

impl Scope {
    pub fn id(&self) -> String {
        match self {
            Self::Mine => MINE.to_owned(),
            Self::Shared => SHARED.to_owned(),
            Self::Recent => RECENT.to_owned(),
            Self::Public => PUBLIC.to_owned(),
            Self::Company(id) => format!("{COMPANY}/{id}"),
            Self::Team(id) => format!("{TEAM}/{id}"),
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            MINE => Some(Self::Mine),
            SHARED => Some(Self::Shared),
            RECENT => Some(Self::Recent),
            PUBLIC => Some(Self::Public),
            other => {
                let (kind, id) = other.split_once('/')?;
                if id.is_empty() {
                    return None;
                }
                match kind {
                    COMPANY => Some(Self::Company(id.to_owned())),
                    TEAM => Some(Self::Team(id.to_owned())),
                    _ => None,
                }
            }
        }
    }

    pub fn query(&self) -> String {
        match self {
            Self::Mine => "filter=0".to_owned(),
            Self::Shared => "filter=2".to_owned(),
            Self::Public => "filter=4".to_owned(),
            Self::Recent => "filter=5".to_owned(),
            Self::Company(id) => format!("filter=7&owner={}&ownerType=1", percent_encode(id)),
            Self::Team(id) => format!("filter=9&owner={}", percent_encode(id)),
        }
    }

    pub fn is_ordered_by_server(&self) -> bool {
        matches!(self, Self::Recent)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceKind {
    Workspace,
    Version,
}

impl PlaceKind {
    fn letter(self) -> &'static str {
        match self {
            Self::Workspace => "w",
            Self::Version => "v",
        }
    }

    fn from_letter(text: &str) -> Option<Self> {
        [Self::Workspace, Self::Version]
            .into_iter()
            .find(|kind| kind.letter() == text)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub kind: PlaceKind,
    pub id: String,
}

impl Place {
    pub fn workspace(id: impl Into<String>) -> Self {
        Self {
            kind: PlaceKind::Workspace,
            id: id.into(),
        }
    }

    pub fn version(id: impl Into<String>) -> Self {
        Self {
            kind: PlaceKind::Version,
            id: id.into(),
        }
    }

    pub fn path(&self) -> String {
        format!("{}/{}", self.kind.letter(), percent_encode(&self.id))
    }

    pub fn folder_id(&self) -> FolderId {
        FolderId::new(format!("{}/{}", self.kind.letter(), self.id))
    }

    pub fn from_folder_id(folder: &FolderId) -> Option<Self> {
        let (letter, id) = folder.as_str().split_once('/')?;
        let kind = PlaceKind::from_letter(letter)?;
        (!id.is_empty() && !id.contains('/')).then(|| Self {
            kind,
            id: id.to_owned(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementKind {
    PartStudio,
    Assembly,
}

impl ElementKind {
    pub fn from_type(text: &str) -> Option<Self> {
        match text {
            "PARTSTUDIO" => Some(Self::PartStudio),
            "ASSEMBLY" => Some(Self::Assembly),
            _ => None,
        }
    }

    pub fn path(self) -> &'static str {
        match self {
            Self::PartStudio => "partstudios",
            Self::Assembly => "assemblies",
        }
    }

    fn from_path(text: &str) -> Option<Self> {
        [Self::PartStudio, Self::Assembly]
            .into_iter()
            .find(|kind| kind.path() == text)
    }
}

pub struct DesignPlace {
    pub kind: ElementKind,
    pub place: Place,
    pub element: String,
}

impl DesignPlace {
    pub fn encode(kind: ElementKind, place: &Place, element: &str) -> DesignId {
        DesignId::new(format!(
            "{}/{}/{}/{element}",
            kind.path(),
            place.kind.letter(),
            place.id
        ))
    }

    pub fn parse(id: &DesignId) -> Option<Self> {
        let mut parts = id.as_str().split('/');
        let kind = ElementKind::from_path(parts.next()?)?;
        let place_kind = PlaceKind::from_letter(parts.next()?)?;
        let place = parts.next()?;
        let element = parts.next()?;
        (parts.next().is_none() && !place.is_empty() && !element.is_empty()).then(|| Self {
            kind,
            place: Place {
                kind: place_kind,
                id: place.to_owned(),
            },
            element: element.to_owned(),
        })
    }
}
