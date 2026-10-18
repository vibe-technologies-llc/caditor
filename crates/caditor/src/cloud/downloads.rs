use std::{
    fs, io,
    path::{Path, PathBuf},
    time::SystemTime,
};

use super::state::ImportItem;

const MAX_NAME_CHARS: usize = 80;
const FALLBACK_NAME: &str = "design";
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0100_0000_01b3;
pub const KEPT_DESIGNS: usize = 40;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub designs: usize,
    pub bytes: u64,
}

pub fn path_for(dir: &Path, item: &ImportItem) -> PathBuf {
    dir.join(format!("{:016x}", identity(item))).join(format!(
        "{}.{}",
        file_stem(&item.stem()),
        item.extension()
    ))
}

pub fn temporary_for(target: &Path) -> PathBuf {
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    target.with_file_name(format!(".{name}.part"))
}

fn identity(item: &ImportItem) -> u64 {
    item.project()
        .as_str()
        .bytes()
        .chain(std::iter::once(0))
        .chain(item.key().into_bytes())
        .fold(FNV_OFFSET, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(FNV_PRIME)
        })
}

fn file_stem(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|symbol| {
            if symbol.is_control() || "/\\:*?\"<>|".contains(symbol) {
                '_'
            } else {
                symbol
            }
        })
        .take(MAX_NAME_CHARS)
        .collect();
    let trimmed = cleaned.trim_matches(|symbol: char| symbol == '.' || symbol.is_whitespace());
    if trimmed.is_empty() {
        FALLBACK_NAME.to_owned()
    } else {
        trimmed.to_owned()
    }
}

pub fn usage(dir: &Path) -> io::Result<Usage> {
    let mut usage = Usage::default();
    for design in design_folders(dir)? {
        usage.designs += 1;
        usage.bytes = usage.bytes.saturating_add(folder_bytes(&design.path));
    }
    Ok(usage)
}

pub fn clear(dir: &Path) -> io::Result<()> {
    for design in design_folders(dir)? {
        fs::remove_dir_all(&design.path)?;
    }
    Ok(())
}

pub fn prune(dir: &Path, keep: usize) -> io::Result<()> {
    let mut designs = design_folders(dir)?;
    designs.sort_by_key(|design| std::cmp::Reverse(design.modified));
    for stale in designs.into_iter().skip(keep) {
        fs::remove_dir_all(&stale.path)?;
    }
    Ok(())
}

struct DesignFolder {
    path: PathBuf,
    modified: Option<SystemTime>,
}

fn design_folders(dir: &Path) -> io::Result<Vec<DesignFolder>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    Ok(entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| DesignFolder {
            modified: entry.metadata().and_then(|meta| meta.modified()).ok(),
            path: entry.path(),
        })
        .collect())
}

fn folder_bytes(folder: &Path) -> u64 {
    fs::read_dir(folder)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| entry.metadata().ok())
        .filter(fs::Metadata::is_file)
        .fold(0, |bytes, meta| bytes.saturating_add(meta.len()))
}

#[cfg(test)]
mod tests {
    use caditor_cloud::{
        Design, DesignId, ProjectId, Upload, UploadFormat, UploadId, Version, VersionId,
    };
    use tempfile::TempDir;

    use super::*;

    fn design(id: &str, name: &str) -> ImportItem {
        ImportItem::Design(Design {
            id: DesignId::new(id),
            project: ProjectId::new("p"),
            name: name.to_owned(),
            modified: None,
        })
    }

    fn upload(id: &str, name: &str, format: UploadFormat) -> ImportItem {
        ImportItem::Upload(Upload {
            id: UploadId::new(id),
            project: ProjectId::new("p"),
            name: name.to_owned(),
            format,
            modified: None,
        })
    }

    fn keep(dir: &Path, design: &ImportItem, bytes: usize) -> PathBuf {
        let path = path_for(dir, design);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, vec![0_u8; bytes]).unwrap();
        path
    }

    #[test]
    fn a_design_is_kept_under_its_own_folder_with_its_own_name() {
        let dir = Path::new("downloads");

        let path = path_for(dir, &design("urn:i:1", "Frame v3"));

        assert_eq!(path.file_name().unwrap(), "Frame v3.step");
        assert_eq!(path.parent().unwrap().parent().unwrap(), dir);
        assert_eq!(path, path_for(dir, &design("urn:i:1", "Frame v3")));
        assert_ne!(
            path.parent(),
            path_for(dir, &design("urn:i:2", "Frame v3")).parent()
        );
    }

    #[test]
    fn an_uploaded_file_keeps_its_own_name_and_kind_under_a_folder_of_its_own() {
        let dir = Path::new("downloads");

        let stl = path_for(dir, &upload("urn:i:1", "Bracket.STL", UploadFormat::Stl));
        let stp = path_for(dir, &upload("urn:i:2", "Plate.stp", UploadFormat::Step));

        assert_eq!(stl.file_name().unwrap(), "Bracket.stl");
        assert_eq!(stp.file_name().unwrap(), "Plate.step");
        assert_ne!(stl.parent(), stp.parent());
    }

    #[test]
    fn a_chosen_version_is_kept_apart_from_the_newest_one_and_named_for_its_number() {
        let dir = Path::new("downloads");
        let frame = |item: &ImportItem| path_for(dir, item);
        let version = |number: u32| {
            ImportItem::Version(
                Design {
                    id: DesignId::new("urn:i:1"),
                    project: ProjectId::new("p"),
                    name: "Frame".to_owned(),
                    modified: None,
                },
                Version {
                    id: VersionId::new(format!("v{number}")),
                    number,
                    modified: None,
                },
            )
        };

        assert_eq!(frame(&version(3)).file_name().unwrap(), "Frame v3.step");
        assert_ne!(frame(&version(3)).parent(), frame(&version(2)).parent());
        assert_ne!(
            frame(&version(3)).parent(),
            frame(&design("urn:i:1", "Frame")).parent()
        );
    }

    #[test]
    fn unsafe_names_are_made_safe_and_empty_ones_get_a_fallback() {
        let stem = |name: &str| {
            path_for(Path::new("d"), &design("i", name))
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        };

        assert_eq!(stem("a/b\\c:d*e?f\"g<h>i|j"), "a_b_c_d_e_f_g_h_i_j");
        assert_eq!(stem("  ..  "), "design");
        assert_eq!(stem(""), "design");
        assert_eq!(stem(&"x".repeat(200)).chars().count(), MAX_NAME_CHARS);
        assert_eq!(stem("tab\there"), "tab_here");
    }

    #[test]
    fn the_temporary_sits_beside_the_target_and_is_hidden() {
        let target = Path::new("d/abc/Frame.step");

        assert_eq!(temporary_for(target), Path::new("d/abc/.Frame.step.part"));
    }

    #[test]
    fn usage_counts_designs_and_bytes_and_clearing_removes_them() {
        let dir = TempDir::new().unwrap();
        keep(dir.path(), &design("1", "A"), 10);
        keep(dir.path(), &design("2", "B"), 25);

        assert_eq!(
            usage(dir.path()).unwrap(),
            Usage {
                designs: 2,
                bytes: 35
            }
        );

        clear(dir.path()).unwrap();

        assert_eq!(usage(dir.path()).unwrap(), Usage::default());
    }

    #[test]
    fn a_missing_folder_is_empty() {
        let dir = TempDir::new().unwrap();

        assert_eq!(usage(&dir.path().join("none")).unwrap(), Usage::default());
        clear(&dir.path().join("none")).unwrap();
        prune(&dir.path().join("none"), 1).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn pruning_keeps_the_newest_designs() {
        let dir = TempDir::new().unwrap();
        let old = keep(dir.path(), &design("old", "Old"), 1);
        let aged = SystemTime::now() - std::time::Duration::from_secs(3600);
        fs::File::open(old.parent().unwrap())
            .unwrap()
            .set_modified(aged)
            .unwrap();
        let fresh = keep(dir.path(), &design("fresh", "Fresh"), 1);

        prune(dir.path(), 1).unwrap();

        assert!(fresh.exists());
        assert!(!old.exists());
    }
}
