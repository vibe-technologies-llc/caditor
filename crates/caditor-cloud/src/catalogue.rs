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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Folder {
    pub folders: Vec<FolderEntry>,
    pub designs: Vec<Design>,
}

impl Folder {
    pub fn merge(&mut self, other: Self) {
        self.folders.extend(other.folders);
        self.designs.extend(other.designs);
    }

    pub fn newest_first(&mut self) {
        self.designs.sort_by(|first, second| {
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
