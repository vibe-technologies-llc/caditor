use std::{fs, path::Path};

use crate::{
    error::NoticesError,
    metadata::{Metadata, Package},
};

const ROOT_PACKAGE: &str = "caditor";
const LICENSE_FILE_PREFIXES: [&str; 6] = [
    "LICENSE",
    "LICENCE",
    "COPYING",
    "UNLICENSE",
    "NOTICE",
    "COPYRIGHT",
];
const MIT_TERMS: &str = include_str!("../licenses/MIT-terms.txt");
const APACHE: &str = include_str!("../licenses/Apache-2.0.txt");
const BOOST: &str = include_str!("../licenses/BSL-1.0.txt");
const STANDARD_LIBRARY: &str = "Rust standard library";
const STANDARD_LIBRARY_LICENSE: &str = "MIT OR Apache-2.0";
const STANDARD_LIBRARY_AUTHORS: &str = "The Rust Project Developers";

struct Asset {
    name: &'static str,
    license: &'static str,
    package: &'static str,
    files: &'static [&'static str],
}

const ASSETS: [Asset; 6] = [
    Asset {
        name: "Inter typeface (interface text)",
        license: "OFL-1.1",
        package: ROOT_PACKAGE,
        files: &["assets/fonts/Inter-LICENSE.txt"],
    },
    Asset {
        name: "Phosphor icon font (interface icons, through egui-phosphor)",
        license: "MIT",
        package: ROOT_PACKAGE,
        files: &["assets/fonts/Phosphor-LICENSE.txt"],
    },
    Asset {
        name: "Hack typeface (egui's default monospace font)",
        license: "MIT AND Bitstream Vera",
        package: "epaint_default_fonts",
        files: &["fonts/Hack-Regular.txt"],
    },
    Asset {
        name: "Ubuntu Light typeface (egui's default text font)",
        license: "Ubuntu-font-1.0",
        package: "epaint_default_fonts",
        files: &["fonts/UFL.txt"],
    },
    Asset {
        name: "Noto Emoji typeface (egui's default emoji font)",
        license: "OFL-1.1",
        package: "epaint_default_fonts",
        files: &["fonts/OFL.txt"],
    },
    Asset {
        name: "emoji-icon-font (egui's default symbol font)",
        license: "MIT",
        package: "epaint_default_fonts",
        files: &["fonts/emoji-icon-font-mit-license.txt"],
    },
];

#[derive(Debug, PartialEq, Eq)]
pub struct Component {
    pub name: String,
    pub license: String,
    pub texts: Vec<String>,
}

pub struct Inventory {
    pub assets: Vec<Component>,
    pub crates: Vec<Component>,
}

pub fn inventory(metadata: &Metadata) -> Result<Inventory, NoticesError> {
    let assets = ASSETS
        .iter()
        .map(|asset| asset_component(metadata, asset))
        .collect::<Result<_, _>>()?;

    let mut crates = metadata
        .linked_from(ROOT_PACKAGE)?
        .into_iter()
        .map(crate_component)
        .collect::<Result<Vec<_>, _>>()?;
    crates.push(standard_library()?);

    Ok(Inventory { assets, crates })
}

fn asset_component(metadata: &Metadata, asset: &Asset) -> Result<Component, NoticesError> {
    let directory = metadata.package_named(asset.package)?.directory();
    let texts = asset
        .files
        .iter()
        .map(|file| read_text(&directory.join(file)))
        .collect::<Result<_, _>>()?;
    Ok(Component {
        name: asset.name.to_owned(),
        license: asset.license.to_owned(),
        texts,
    })
}

fn crate_component(package: &Package) -> Result<Component, NoticesError> {
    let name = format!("{} {}", package.name, package.version);
    let license = package.license.clone().unwrap_or_default();
    let mut texts = license_files(package.directory())?;
    if texts.is_empty() {
        let holders = copyright_holders(package);
        texts = standard_texts(&license, &holders);
    }
    if texts.is_empty() {
        return Err(NoticesError::NoLicenseText {
            package: name,
            license,
        });
    }
    Ok(Component {
        name,
        license,
        texts,
    })
}

fn standard_library() -> Result<Component, NoticesError> {
    let texts = standard_texts(STANDARD_LIBRARY_LICENSE, STANDARD_LIBRARY_AUTHORS);
    if texts.is_empty() {
        return Err(NoticesError::NoLicenseText {
            package: STANDARD_LIBRARY.to_owned(),
            license: STANDARD_LIBRARY_LICENSE.to_owned(),
        });
    }
    Ok(Component {
        name: STANDARD_LIBRARY.to_owned(),
        license: STANDARD_LIBRARY_LICENSE.to_owned(),
        texts,
    })
}

fn license_files(directory: &Path) -> Result<Vec<String>, NoticesError> {
    let unreadable = |source| NoticesError::Unreadable {
        path: directory.to_path_buf(),
        source,
    };
    let mut paths = Vec::new();
    for entry in fs::read_dir(directory).map_err(unreadable)? {
        let path = entry.map_err(unreadable)?.path();
        let upper = path
            .file_name()
            .map(|name| name.to_string_lossy().to_uppercase())
            .unwrap_or_default();
        let is_license = LICENSE_FILE_PREFIXES
            .iter()
            .any(|prefix| upper.starts_with(prefix));
        if is_license && path.is_file() {
            paths.push(path);
        }
    }
    paths.sort();
    paths.iter().map(|path| read_text(path)).collect()
}

fn read_text(path: &Path) -> Result<String, NoticesError> {
    let bytes = fs::read(path).map_err(|source| NoticesError::Unreadable {
        path: path.to_path_buf(),
        source,
    })?;
    let text = String::from_utf8(bytes).map_err(|_| NoticesError::NotText {
        path: path.to_path_buf(),
    })?;
    Ok(normalized(&text))
}

pub fn normalized(text: &str) -> String {
    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    lines.join("\n").trim_matches('\n').to_owned()
}

fn copyright_holders(package: &Package) -> String {
    let named: Vec<&str> = package
        .authors
        .iter()
        .map(|author| author.split('<').next().unwrap_or(author).trim())
        .filter(|author| !author.is_empty())
        .collect();
    if named.is_empty() {
        format!("the authors of {}", package.name)
    } else {
        named.join(", ")
    }
}

fn standard_texts(license: &str, holders: &str) -> Vec<String> {
    let identifiers: Vec<&str> = license
        .split(|character: char| character.is_whitespace() || matches!(character, '(' | ')' | '/'))
        .collect();
    let named = |identifier: &str| identifiers.contains(&identifier);

    let mut texts = Vec::new();
    if named("MIT") {
        let mit = format!("MIT License\n\nCopyright (c) {holders}\n\n{MIT_TERMS}");
        texts.push(normalized(&mit));
    }
    if named("Apache-2.0") {
        texts.push(normalized(APACHE));
    }
    if named("BSL-1.0") {
        texts.push(normalized(BOOST));
    }
    texts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_endings_and_trailing_blanks_do_not_tell_two_copies_apart() {
        let windows = "\r\n\r\nPermission  \r\n granted.\r\n\r\n";
        let unix = "Permission\n granted.\n";

        assert_eq!(normalized(windows), normalized(unix));
        assert_eq!(normalized(unix), "Permission\n granted.");
    }

    #[test]
    fn a_licence_without_a_file_is_written_out_for_each_standard_text_it_names() {
        let both = standard_texts("MIT OR Apache-2.0", "A. Author");
        let nested = standard_texts("(MIT OR Apache-2.0) AND OFL-1.1", "A. Author");
        let boost = standard_texts("BSL-1.0", "ignored");
        let unknown = standard_texts("GPL-3.0-only", "A. Author");

        assert_eq!(both.len(), 2);
        assert!(
            both.first()
                .is_some_and(|mit| mit.starts_with("MIT License\n\nCopyright (c) A. Author\n\n"))
        );
        assert!(
            both.get(1)
                .is_some_and(|apache| apache.contains("Apache License"))
        );
        assert_eq!(nested, both);
        assert_eq!(boost.len(), 1);
        assert!(
            boost
                .first()
                .is_some_and(|text| text.starts_with("Boost Software License"))
        );
        assert!(unknown.is_empty());
    }

    #[test]
    fn copyright_holders_are_the_authors_without_their_addresses() {
        let metadata = crate::metadata::parse(
            br#"{"packages": [{"id": "a", "name": "a", "version": "1.0.0", "license": "MIT",
            "authors": ["Emil E <emil@example.org>", "Second Author", "<only@example.org>"],
            "source": "registry+x", "manifest_path": "/r/a/Cargo.toml", "targets": []},
            {"id": "b", "name": "b", "version": "1.0.0", "license": "MIT", "authors": [],
            "source": "registry+x", "manifest_path": "/r/b/Cargo.toml", "targets": []}],
            "resolve": {"nodes": []}}"#,
        )
        .unwrap();

        let named = copyright_holders(metadata.package_named("a").unwrap());
        let anonymous = copyright_holders(metadata.package_named("b").unwrap());

        assert_eq!(named, "Emil E, Second Author");
        assert_eq!(anonymous, "the authors of b");
    }
}
