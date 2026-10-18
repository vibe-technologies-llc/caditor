use jiff::Timestamp;

macro_rules! identifier {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            pub fn new(text: impl Into<String>) -> Self {
                Self(text.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

identifier!(WorkspaceId);
identifier!(ProjectId);
identifier!(FolderId);
identifier!(DesignId);
identifier!(UploadId);
identifier!(VersionId);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceKind {
    Team,
    Personal,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub name: String,
    pub kind: WorkspaceKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub id: ProjectId,
    pub workspace: WorkspaceId,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FolderRef {
    pub project: ProjectId,
    pub folder: FolderId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderEntry {
    pub reference: FolderRef,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Design {
    pub id: DesignId,
    pub project: ProjectId,
    pub name: String,
    pub modified: Option<Timestamp>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UploadFormat {
    Step,
    Stl,
    Obj,
    ThreeMf,
    Dxf,
    Svg,
}

impl UploadFormat {
    pub fn of_file_name(name: &str) -> Option<Self> {
        let (_, extension) = name.trim().rsplit_once('.')?;
        match extension.to_ascii_lowercase().as_str() {
            "step" | "stp" => Some(Self::Step),
            "stl" => Some(Self::Stl),
            "obj" => Some(Self::Obj),
            "3mf" => Some(Self::ThreeMf),
            "dxf" => Some(Self::Dxf),
            "svg" => Some(Self::Svg),
            _ => None,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Step => "step",
            Self::Stl => "stl",
            Self::Obj => "obj",
            Self::ThreeMf => "3mf",
            Self::Dxf => "dxf",
            Self::Svg => "svg",
        }
    }

    pub fn is_drawing(self) -> bool {
        matches!(self, Self::Dxf | Self::Svg)
    }

    pub fn is_mesh(self) -> bool {
        matches!(self, Self::Stl | Self::Obj | Self::ThreeMf)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upload {
    pub id: UploadId,
    pub project: ProjectId,
    pub name: String,
    pub format: UploadFormat,
    pub modified: Option<Timestamp>,
}

impl Upload {
    pub fn stem(&self) -> &str {
        let name = self.name.trim();
        match name.rsplit_once('.') {
            Some((stem, _)) if !stem.is_empty() => stem,
            _ => name,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Folder {
    pub place: Option<FolderRef>,
    pub folders: Vec<FolderEntry>,
    pub designs: Vec<Design>,
    pub uploads: Vec<Upload>,
}

impl Folder {
    pub fn merge(&mut self, other: Self) {
        self.folders.extend(other.folders);
        self.designs.extend(other.designs);
        self.uploads.extend(other.uploads);
    }

    pub fn newest_first(&mut self) {
        self.designs.sort_by(|first, second| {
            second
                .modified
                .cmp(&first.modified)
                .then_with(|| first.name.cmp(&second.name))
        });
        self.uploads.sort_by(|first, second| {
            second
                .modified
                .cmp(&first.modified)
                .then_with(|| first.name.cmp(&second.name))
        });
        self.folders
            .sort_by(|first, second| first.name.cmp(&second.name));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub id: VersionId,
    pub number: u32,
    pub modified: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Translation {
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Download {
    pub source: String,
    pub artifact: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conversion {
    Working { percent: Option<u8> },
    Ready(Download),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outgoing<'a> {
    pub folder: &'a FolderRef,
    pub name: &'a str,
    pub bytes: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendOutcome {
    Created,
    NewVersion { number: Option<u32> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    pub name: String,
    pub outcome: SendOutcome,
}
