use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
};

use serde::Deserialize;

use crate::error::NoticesError;

const PROC_MACRO: &str = "proc-macro";

#[derive(Debug, Deserialize)]
pub struct Metadata {
    packages: Vec<Package>,
    resolve: Resolve,
}

#[derive(Debug, Deserialize)]
pub struct Package {
    id: String,
    pub name: String,
    pub version: String,
    pub license: Option<String>,
    pub authors: Vec<String>,
    source: Option<String>,
    manifest_path: PathBuf,
    targets: Vec<Target>,
}

#[derive(Debug, Deserialize)]
struct Target {
    kind: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Resolve {
    nodes: Vec<Node>,
}

#[derive(Debug, Deserialize)]
struct Node {
    id: String,
    deps: Vec<Dependency>,
}

#[derive(Debug, Deserialize)]
struct Dependency {
    pkg: String,
    dep_kinds: Vec<DependencyKind>,
}

#[derive(Debug, Deserialize)]
struct DependencyKind {
    kind: Option<String>,
}

impl Dependency {
    fn is_linked(&self) -> bool {
        self.dep_kinds.iter().any(|kind| kind.kind.is_none())
    }
}

impl Package {
    fn is_proc_macro(&self) -> bool {
        self.targets
            .iter()
            .any(|target| target.kind.iter().any(|kind| kind == PROC_MACRO))
    }

    pub fn is_external(&self) -> bool {
        self.source.is_some()
    }

    pub fn directory(&self) -> &Path {
        self.manifest_path.parent().unwrap_or(Path::new(""))
    }
}

pub fn read(target: Option<&str>) -> Result<Metadata, NoticesError> {
    let cargo = env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let mut command = Command::new(cargo);
    command.args(["metadata", "--format-version", "1", "--locked"]);
    if let Some(target) = target {
        command.args(["--filter-platform", target]);
    }
    let output = command.output().map_err(NoticesError::CargoNotStarted)?;
    if !output.status.success() {
        return Err(NoticesError::MetadataFailed {
            status: output.status,
            report: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    parse(&output.stdout)
}

pub fn parse(json: &[u8]) -> Result<Metadata, NoticesError> {
    serde_json::from_slice(json).map_err(NoticesError::MetadataUnreadable)
}

impl Metadata {
    pub fn package_named(&self, name: &str) -> Result<&Package, NoticesError> {
        self.packages
            .iter()
            .find(|package| package.name == name)
            .ok_or_else(|| NoticesError::PackageMissing {
                package: name.to_owned(),
            })
    }

    pub fn linked_from(&self, root: &str) -> Result<Vec<&Package>, NoticesError> {
        let packages: BTreeMap<&str, &Package> = self
            .packages
            .iter()
            .map(|package| (package.id.as_str(), package))
            .collect();
        let nodes: BTreeMap<&str, &Node> = self
            .resolve
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), node))
            .collect();

        let mut reached = BTreeSet::new();
        let mut pending = vec![self.package_named(root)?.id.as_str()];
        while let Some(id) = pending.pop() {
            if !reached.insert(id) {
                continue;
            }
            let Some(node) = nodes.get(id) else {
                continue;
            };
            let linked = node
                .deps
                .iter()
                .filter(|dependency| dependency.is_linked())
                .filter_map(|dependency| packages.get(dependency.pkg.as_str()))
                .filter(|package| !package.is_proc_macro());
            pending.extend(linked.map(|package| package.id.as_str()));
        }

        let mut linked: Vec<&Package> = reached
            .into_iter()
            .filter_map(|id| packages.get(id).copied())
            .filter(|package| package.is_external())
            .collect();
        linked
            .sort_by(|left, right| (&left.name, &left.version).cmp(&(&right.name, &right.version)));
        Ok(linked)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRAPH: &str = r#"{
        "packages": [
            {"id": "app", "name": "app", "version": "0.1.0", "license": "AGPL-3.0-only",
             "authors": [], "source": null, "manifest_path": "/work/app/Cargo.toml",
             "targets": [{"kind": ["bin"]}]},
            {"id": "lib", "name": "lib", "version": "1.0.0", "license": "MIT",
             "authors": ["A <a@example.org>"], "source": "registry+x",
             "manifest_path": "/reg/lib-1.0.0/Cargo.toml", "targets": [{"kind": ["lib"]}]},
            {"id": "derive", "name": "derive", "version": "1.0.0", "license": "MIT",
             "authors": [], "source": "registry+x", "manifest_path": "/reg/derive/Cargo.toml",
             "targets": [{"kind": ["proc-macro"]}]},
            {"id": "syn", "name": "syn", "version": "2.0.0", "license": "MIT",
             "authors": [], "source": "registry+x", "manifest_path": "/reg/syn/Cargo.toml",
             "targets": [{"kind": ["lib"]}]},
            {"id": "build-tool", "name": "build-tool", "version": "1.0.0", "license": "MIT",
             "authors": [], "source": "registry+x", "manifest_path": "/reg/b/Cargo.toml",
             "targets": [{"kind": ["lib"]}]},
            {"id": "deep", "name": "deep", "version": "3.0.0", "license": "MIT",
             "authors": [], "source": "registry+x", "manifest_path": "/reg/deep/Cargo.toml",
             "targets": [{"kind": ["lib"]}]}
        ],
        "resolve": {"nodes": [
            {"id": "app", "deps": [
                {"pkg": "lib", "dep_kinds": [{"kind": null}]},
                {"pkg": "derive", "dep_kinds": [{"kind": null}]},
                {"pkg": "build-tool", "dep_kinds": [{"kind": "build"}]}]},
            {"id": "lib", "deps": [{"pkg": "deep", "dep_kinds": [{"kind": null}, {"kind": "dev"}]}]},
            {"id": "derive", "deps": [{"pkg": "syn", "dep_kinds": [{"kind": null}]}]},
            {"id": "syn", "deps": []},
            {"id": "build-tool", "deps": []},
            {"id": "deep", "deps": []}
        ]}
    }"#;

    #[test]
    fn only_the_crates_linked_into_the_program_are_reached() {
        let metadata = parse(GRAPH.as_bytes()).unwrap();

        let linked = metadata.linked_from("app").unwrap();
        let names: Vec<&str> = linked.iter().map(|package| package.name.as_str()).collect();

        assert_eq!(names, ["deep", "lib"]);
    }

    #[test]
    fn a_root_that_is_not_in_the_graph_is_named() {
        let metadata = parse(GRAPH.as_bytes()).unwrap();

        let error = metadata.linked_from("elsewhere").unwrap_err();

        assert!(
            matches!(error, NoticesError::PackageMissing { package } if package == "elsewhere")
        );
    }

    #[test]
    fn unreadable_metadata_is_an_error() {
        assert!(matches!(
            parse(b"not json"),
            Err(NoticesError::MetadataUnreadable(_))
        ));
    }
}
