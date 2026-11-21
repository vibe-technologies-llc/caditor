use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use super::{
    MANIFEST, Member, OUTSIDE_READS, Touch, dependents_listed, members, moves_dependencies, owners,
    package_name, within,
};
use crate::run;

const INCLUDES: [&str; 2] = ["include_bytes!(\"", "include_str!(\""];
const FROM_THE_MANIFEST: &str = "env!(\"CARGO_MANIFEST_DIR\")).join(\"";
const WHOLE_TREE_READER: &str = "crates/caditor/src/conventions_tests.rs";

fn member(name: &str, folder: &str) -> Member {
    Member {
        name: name.to_owned(),
        folder: folder.to_owned(),
    }
}

fn files(listed: &[&str]) -> BTreeSet<String> {
    listed.iter().map(|file| (*file).to_owned()).collect()
}

fn example_members() -> Vec<Member> {
    vec![
        member("caditor", "crates/caditor"),
        member("caditor-file", "crates/caditor-file"),
        member("caditor-lettering", "crates/caditor-lettering"),
        member("caditor-kernel", "crates/caditor-kernel"),
    ]
}

#[test]
fn a_manifest_names_its_package_from_the_package_table_alone() {
    let own = "[package]\nname = \"caditor-kernel\"\nversion.workspace = true\n\n[dependencies]\nname = \"other\"\n";
    let late = "[lints]\nworkspace = true\n\n[package]\nversion = \"1.0.0\"\nname = \"late\"\n";

    assert_eq!(package_name(own), Some("caditor-kernel"));
    assert_eq!(package_name(late), Some("late"));
    assert_eq!(package_name("[workspace]\nname = \"no\"\n"), None);
}

#[test]
fn a_file_belongs_to_the_member_whose_folder_holds_it_and_not_one_sharing_its_prefix() {
    let touched = owners(
        &files(&[
            "crates/caditor-file/src/lib.rs",
            "crates/caditor-kernel/clippy.toml",
            "docs/BUGS.md",
            ".claude/rules/kernel.md",
        ]),
        &[
            member("caditor", "crates/caditor"),
            member("caditor-file", "crates/caditor-file"),
            member("caditor-kernel", "crates/caditor-kernel"),
        ],
    );

    assert_eq!(
        touched,
        Touch::These(files(&["caditor-file", "caditor-kernel"]))
    );
}

#[test]
fn a_file_another_crate_reads_touches_that_crate_too() {
    let font = owners(
        &files(&["crates/caditor/assets/fonts/InterVariable.ttf"]),
        &example_members(),
    );
    let seed = owners(
        &files(&["fuzz/seeds/step/horn_torus.step"]),
        &example_members(),
    );
    let target = owners(&files(&["fuzz/fuzz_targets/step.rs"]), &example_members());

    assert_eq!(
        font,
        Touch::These(files(&["caditor", "caditor-file", "caditor-lettering"]))
    );
    assert_eq!(seed, Touch::These(files(&["caditor-file"])));
    assert_eq!(target, Touch::These(BTreeSet::new()));
}

#[test]
fn a_workspace_wide_file_touches_every_package() {
    let lock = owners(&files(&["Cargo.lock"]), &example_members());
    let nextest = owners(&files(&[".config/nextest.toml"]), &example_members());
    let lints = owners(&files(&["clippy.toml"]), &example_members());
    let inner = owners(&files(&["crates/caditor/Cargo.toml"]), &example_members());

    assert_eq!(lock, Touch::Everything);
    assert_eq!(nextest, Touch::Everything);
    assert_eq!(lints, Touch::Everything);
    assert_eq!(inner, Touch::These(files(&["caditor"])));
}

#[test]
fn deny_is_wanted_when_a_manifest_a_lock_or_its_configuration_changed() {
    assert!(moves_dependencies(&files(&["crates/caditor/Cargo.toml"])));
    assert!(moves_dependencies(&files(&["fuzz/Cargo.lock"])));
    assert!(moves_dependencies(&files(&["deny.toml"])));
    assert!(!moves_dependencies(&files(&[
        "crates/caditor/src/app.rs",
        "docs/deny.toml.md"
    ])));
}

#[test]
fn the_inverted_tree_is_read_for_the_workspace_s_own_packages_alone() {
    let listed = "caditor-lettering v0.1.0 (/work/caditor/crates/caditor-lettering)\n\
                  caditor-file v0.1.0 (/work/caditor/crates/caditor-file) (*)\n\
                  caditor-file v0.1.0 (/work/caditor/crates/caditor-file)\n\
                  skrifa v0.50.0\n";

    assert_eq!(
        dependents_listed(listed, &example_members()),
        files(&["caditor-file", "caditor-lettering"])
    );
}

fn rust_files(folder: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(folder).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, found);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            found.push(path);
        }
    }
}

fn literals_after<'a>(source: &'a str, opening: &'a str) -> impl Iterator<Item = &'a str> {
    source
        .match_indices(opening)
        .filter_map(move |(at, _)| source.get(at + opening.len()..)?.split('"').next())
}

fn resolved(base: &str, literal: &str) -> String {
    let mut segments: Vec<&str> = base.split('/').filter(|part| !part.is_empty()).collect();

    for part in literal.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            name => segments.push(name),
        }
    }
    segments.join("/")
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap()
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

fn depends_on(manifest: &str, package: &str) -> bool {
    manifest.lines().any(|line| {
        let line = line.trim();
        line.starts_with(&format!("{package}.workspace"))
            || line.starts_with(&format!("{package} ="))
    })
}

#[test]
fn every_file_a_crate_reads_outside_its_folder_selects_it_when_changed() {
    let root = run::workspace_root();
    let members = members(&root).unwrap();
    let mut unlisted = Vec::new();

    assert!(members.len() > 1);

    for reader in &members {
        let manifest = fs::read_to_string(root.join(&reader.folder).join(MANIFEST)).unwrap();
        let mut sources = Vec::new();

        rust_files(&root.join(&reader.folder), &mut sources);

        for source in sources {
            let text = fs::read_to_string(&source).unwrap();
            let file = relative(&root, &source);
            let folder = file.rsplit_once('/').map_or("", |(folder, _)| folder);

            let included = INCLUDES
                .iter()
                .flat_map(|opening| literals_after(&text, opening))
                .map(|literal| resolved(folder, literal));
            let joined = literals_after(&text, FROM_THE_MANIFEST)
                .map(|literal| resolved(&reader.folder, literal));

            for path in included.chain(joined) {
                let own = within(&path, &reader.folder);
                let conventions = path.is_empty() && file == WHOLE_TREE_READER;
                let depended_on = members.iter().any(|other| {
                    within(&path, &other.folder) && depends_on(&manifest, &other.name)
                });
                let listed = OUTSIDE_READS.iter().any(|(folder, readers)| {
                    within(&path, folder) && readers.contains(&reader.name.as_str())
                });

                if !(own || conventions || depended_on || listed) {
                    unlisted.push(format!("{file} reads {path}"));
                }
            }
        }
    }

    assert!(
        unlisted.is_empty(),
        "add each to OUTSIDE_READS with the crate reading it, so --changed selects that crate \
         when the file changes: {unlisted:#?}"
    );
}

#[test]
fn every_folder_read_from_outside_exists() {
    let root = run::workspace_root();
    let names: BTreeSet<String> = members(&root)
        .unwrap()
        .into_iter()
        .map(|member| member.name)
        .collect();

    for (folder, readers) in OUTSIDE_READS {
        assert!(root.join(folder).is_dir(), "{folder}");
        assert!(
            readers.iter().all(|reader| names.contains(*reader)),
            "{readers:?}"
        );
    }
}
