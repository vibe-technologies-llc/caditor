#[cfg(test)]
mod tests;

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use crate::{
    error::{Error, Query},
    run,
};

pub const CRATES: &str = "crates";
pub const MANIFEST: &str = "Cargo.toml";

const LOCK: &str = "Cargo.lock";
const DENY_CONFIG: &str = "deny.toml";
const WORKSPACE_WIDE_FILES: [&str; 4] = [MANIFEST, LOCK, "clippy.toml", "rust-toolchain.toml"];
const WORKSPACE_WIDE_FOLDERS: [&str; 2] = [".cargo", ".config"];

pub const OUTSIDE_READS: [(&str, &[&str]); 3] = [
    ("packaging", &["caditor", "caditor-file"]),
    ("fuzz/seeds", &["caditor-file"]),
    (
        "crates/caditor/assets/fonts",
        &["caditor-lettering", "caditor-file"],
    ),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub name: String,
    pub folder: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Touch {
    Everything,
    These(BTreeSet<String>),
}

pub fn files(root: &Path, since: &str) -> Result<BTreeSet<String>, Error> {
    let base = run::asked(
        git(root).args(["merge-base", "HEAD", since]),
        "git",
        Query::MergeBase,
    )?;
    let differing = run::asked(
        git(root).args(["diff", "--name-only", base.trim()]),
        "git",
        Query::Differing,
    )?;
    let untracked = run::asked(
        git(root).args(["ls-files", "--others", "--exclude-standard"]),
        "git",
        Query::Untracked,
    )?;

    Ok(differing
        .lines()
        .chain(untracked.lines())
        .filter(|file| !file.is_empty())
        .map(str::to_owned)
        .collect())
}

fn git(root: &Path) -> Command {
    let mut command = Command::new("git");

    command.current_dir(root);
    command
}

pub fn members(root: &Path) -> Result<Vec<Member>, Error> {
    let crates = root.join(CRATES);
    let entries = fs::read_dir(&crates).map_err(|source| Error::Read {
        path: crates.clone(),
        source,
    })?;

    let mut members = Vec::new();

    for entry in entries {
        let entry = entry.map_err(|source| Error::Read {
            path: crates.clone(),
            source,
        })?;
        let path = entry.path().join(MANIFEST);

        if !path.is_file() {
            continue;
        }

        let manifest = fs::read_to_string(&path).map_err(|source| Error::Read {
            path: path.clone(),
            source,
        })?;
        let name = package_name(&manifest).ok_or_else(|| Error::Unnamed {
            path: PathBuf::from(&path),
        })?;

        members.push(Member {
            name: name.to_owned(),
            folder: format!("{CRATES}/{}", entry.file_name().to_string_lossy()),
        });
    }

    members.sort_by(|one, other| one.folder.cmp(&other.folder));
    Ok(members)
}

pub fn package_name(manifest: &str) -> Option<&str> {
    manifest
        .lines()
        .skip_while(|line| line.trim() != "[package]")
        .skip(1)
        .take_while(|line| !line.trim_start().starts_with('['))
        .find_map(|line| line.trim().strip_prefix("name = \"")?.strip_suffix('"'))
}

pub fn within(path: &str, folder: &str) -> bool {
    path.strip_prefix(folder)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

pub fn owners(files: &BTreeSet<String>, members: &[Member]) -> Touch {
    if files.iter().any(|file| is_workspace_wide(file)) {
        return Touch::Everything;
    }

    Touch::These(
        files
            .iter()
            .flat_map(|file| {
                owner(file, members)
                    .map(str::to_owned)
                    .into_iter()
                    .chain(outside_readers(file).map(str::to_owned))
            })
            .collect(),
    )
}

fn is_workspace_wide(file: &str) -> bool {
    WORKSPACE_WIDE_FILES.contains(&file)
        || WORKSPACE_WIDE_FOLDERS
            .iter()
            .any(|folder| within(file, folder))
}

fn owner<'a>(file: &str, members: &'a [Member]) -> Option<&'a str> {
    members
        .iter()
        .find(|member| within(file, &member.folder))
        .map(|member| member.name.as_str())
}

fn outside_readers(file: &str) -> impl Iterator<Item = &'static str> {
    OUTSIDE_READS
        .iter()
        .filter(move |(folder, _)| within(file, folder))
        .flat_map(|(_, readers)| readers.iter().copied())
}

pub fn moves_dependencies(files: &BTreeSet<String>) -> bool {
    files.iter().any(|file| {
        file == DENY_CONFIG
            || file
                .rsplit('/')
                .next()
                .is_some_and(|name| name == MANIFEST || name == LOCK)
    })
}

pub fn dependents(
    root: &Path,
    touched: &BTreeSet<String>,
    members: &[Member],
) -> Result<BTreeSet<String>, Error> {
    let mut command = Command::new("cargo");

    command.current_dir(root).args([
        "tree",
        "--workspace",
        "--target",
        "all",
        "--edges",
        "normal,build,dev",
        "--prefix",
        "none",
        "--format",
        "{p}",
    ]);
    for package in touched {
        command.args(["--invert", package]);
    }

    let listed = run::asked(&mut command, "cargo", Query::Dependents)?;
    let mut found = dependents_listed(&listed, members);

    found.extend(touched.iter().cloned());
    Ok(found)
}

pub fn dependents_listed(listed: &str, members: &[Member]) -> BTreeSet<String> {
    listed
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|name| members.iter().any(|member| member.name == *name))
        .map(str::to_owned)
        .collect()
}
