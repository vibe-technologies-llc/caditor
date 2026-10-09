use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
};

use egui::FontData;

use crate::model::Waker;

const MAX_DEPTH: usize = 5;
const MAX_ENTRIES: usize = 200_000;
const MAX_FONT_BYTES: u64 = 64 * 1024 * 1024;
const FONT_EXTENSIONS: [&str; 3] = ["ttf", "otf", "ttc"];
const KEY_PREFIX: &str = "fallback-";
const COLLECTION_TAG: &[u8; 4] = b"ttcf";
const OUTLINE_VERSIONS: [u32; 3] = [
    0x0001_0000,
    u32::from_be_bytes(*b"OTTO"),
    u32::from_be_bytes(*b"true"),
];
const TABLE_DIRECTORY_HEADER: usize = 12;
const TABLE_RECORD: usize = 16;
const COLLECTION_HEADER: usize = 12;
const COLLECTION_SIGNATURE_FIELDS: usize = 12;

struct Script {
    name: &'static str,
    files: &'static [&'static str],
}

#[cfg(unix)]
const SCRIPTS: &[Script] = &[
    Script {
        name: "Han, kana and hangul",
        files: &[
            "NotoSansCJK-Regular.ttc",
            "NotoSansCJKsc-Regular.otf",
            "NotoSansCJKjp-Regular.otf",
            "NotoSansCJKkr-Regular.otf",
            "NotoSansSC-Regular.otf",
            "NotoSansJP-Regular.otf",
            "SourceHanSans-Regular.ttc",
            "SourceHanSansSC-Regular.otf",
            "wqy-microhei.ttc",
            "wqy-zenhei.ttc",
            "DroidSansFallbackFull.ttf",
            "DroidSansFallback.ttf",
        ],
    },
    Script {
        name: "Arabic",
        files: &[
            "NotoSansArabic-Regular.ttf",
            "NotoNaskhArabic-Regular.ttf",
            "NotoKufiArabic-Regular.ttf",
        ],
    },
    Script {
        name: "Hebrew",
        files: &["NotoSansHebrew-Regular.ttf"],
    },
    Script {
        name: "Devanagari",
        files: &["NotoSansDevanagari-Regular.ttf", "Lohit-Devanagari.ttf"],
    },
    Script {
        name: "Bengali",
        files: &["NotoSansBengali-Regular.ttf", "Lohit-Bengali.ttf"],
    },
    Script {
        name: "Gurmukhi",
        files: &["NotoSansGurmukhi-Regular.ttf", "Lohit-Gurmukhi.ttf"],
    },
    Script {
        name: "Gujarati",
        files: &["NotoSansGujarati-Regular.ttf", "Lohit-Gujarati.ttf"],
    },
    Script {
        name: "Oriya",
        files: &["NotoSansOriya-Regular.ttf", "Lohit-Odia.ttf"],
    },
    Script {
        name: "Tamil",
        files: &["NotoSansTamil-Regular.ttf", "Lohit-Tamil.ttf"],
    },
    Script {
        name: "Telugu",
        files: &["NotoSansTelugu-Regular.ttf", "Lohit-Telugu.ttf"],
    },
    Script {
        name: "Kannada",
        files: &["NotoSansKannada-Regular.ttf", "Lohit-Kannada.ttf"],
    },
    Script {
        name: "Malayalam",
        files: &["NotoSansMalayalam-Regular.ttf", "Lohit-Malayalam.ttf"],
    },
    Script {
        name: "Sinhala",
        files: &["NotoSansSinhala-Regular.ttf"],
    },
    Script {
        name: "Thai",
        files: &["NotoSansThai-Regular.ttf", "TlwgTypo.ttf"],
    },
    Script {
        name: "Lao",
        files: &["NotoSansLao-Regular.ttf"],
    },
    Script {
        name: "Khmer",
        files: &["NotoSansKhmer-Regular.ttf"],
    },
    Script {
        name: "Myanmar",
        files: &["NotoSansMyanmar-Regular.ttf"],
    },
    Script {
        name: "Tibetan",
        files: &["NotoSerifTibetan-Regular.ttf"],
    },
    Script {
        name: "Georgian",
        files: &["NotoSansGeorgian-Regular.ttf"],
    },
    Script {
        name: "Armenian",
        files: &["NotoSansArmenian-Regular.ttf"],
    },
    Script {
        name: "Ethiopic",
        files: &["NotoSansEthiopic-Regular.ttf"],
    },
    Script {
        name: "Symbols",
        files: &[
            "NotoSansSymbols-Regular.ttf",
            "NotoSansSymbols2-Regular.ttf",
        ],
    },
    Script {
        name: "Wide coverage",
        files: &["DejaVuSans.ttf", "FreeSans.ttf"],
    },
];

#[cfg(windows)]
const SCRIPTS: &[Script] = &[
    Script {
        name: "Han and kana",
        files: &[
            "msyh.ttc",
            "YuGothR.ttc",
            "meiryo.ttc",
            "msgothic.ttc",
            "simsun.ttc",
        ],
    },
    Script {
        name: "Hangul",
        files: &["malgun.ttf", "gulim.ttc"],
    },
    Script {
        name: "Arabic, Hebrew, Armenian and Georgian",
        files: &["segoeui.ttf", "arial.ttf"],
    },
    Script {
        name: "Indic scripts",
        files: &["Nirmala.ttc", "Nirmala.ttf", "mangal.ttf"],
    },
    Script {
        name: "Thai, Lao and Khmer",
        files: &["LeelawUI.ttf", "leelawad.ttf", "tahoma.ttf"],
    },
    Script {
        name: "Ethiopic",
        files: &["ebrima.ttf"],
    },
    Script {
        name: "Myanmar",
        files: &["mmrtext.ttf"],
    },
    Script {
        name: "Tibetan",
        files: &["himalaya.ttf"],
    },
    Script {
        name: "Symbols",
        files: &["seguisym.ttf"],
    },
];

#[derive(Debug, Clone)]
pub struct FallbackFont {
    pub key: String,
    pub data: Arc<FontData>,
}

#[derive(Debug, Default)]
pub enum FallbackFonts {
    #[default]
    Off,
    Searching(Receiver<Vec<FallbackFont>>),
    Found(Arc<[FallbackFont]>),
}

impl FallbackFonts {
    pub fn search(wake: Waker) -> Self {
        let (sender, receiver) = mpsc::channel();
        let spawned = thread::Builder::new()
            .name("font search".to_owned())
            .spawn(move || {
                if sender.send(find()).is_ok() {
                    wake();
                }
            });
        match spawned {
            Ok(_) => Self::Searching(receiver),
            Err(error) => {
                log::warn!("fonts for other scripts are not looked for: {error}");
                Self::Off
            }
        }
    }

    pub fn arrived(&mut self) -> bool {
        let Self::Searching(receiver) = self else {
            return false;
        };
        match receiver.try_recv() {
            Ok(found) => {
                let arrived = !found.is_empty();
                *self = Self::Found(found.into());
                arrived
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => {
                *self = Self::Off;
                false
            }
        }
    }

    pub fn found(&self) -> &[FallbackFont] {
        match self {
            Self::Found(found) => found,
            Self::Off | Self::Searching(_) => &[],
        }
    }
}

fn find() -> Vec<FallbackFont> {
    let files = font_files(&font_directories());
    let mut chosen: Vec<&Path> = Vec::new();
    for script in SCRIPTS {
        match script.files.iter().find_map(|name| lookup(&files, name)) {
            Some(path) if !chosen.contains(&path) => chosen.push(path),
            Some(_) => {}
            None => log::debug!("no font found for {}", script.name),
        }
    }
    chosen.into_iter().filter_map(load).collect()
}

fn lookup<'a>(files: &'a BTreeMap<String, PathBuf>, name: &str) -> Option<&'a Path> {
    let name = name.to_lowercase();
    if let Some(path) = files.get(&name) {
        return Some(path);
    }
    let stem = name
        .rsplit_once('.')
        .map_or(name.as_str(), |(stem, _)| stem);
    let family = stem.strip_suffix("-regular")?;
    let variable = format!("{family}[");
    files
        .range(variable.clone()..)
        .next()
        .filter(|(file, _)| file.starts_with(&variable))
        .map(|(_, path)| path.as_path())
}

fn font_files(directories: &[PathBuf]) -> BTreeMap<String, PathBuf> {
    let mut files = BTreeMap::new();
    let mut pending: Vec<(PathBuf, usize)> = directories
        .iter()
        .rev()
        .map(|directory| (directory.clone(), 0))
        .collect();
    let mut visited = 0_usize;
    while let Some((directory, depth)) = pending.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        let mut subdirectories = Vec::new();
        for entry in entries.flatten() {
            visited = visited.saturating_add(1);
            if visited > MAX_ENTRIES {
                log::debug!("stopped looking for fonts after {MAX_ENTRIES} entries");
                return files;
            }
            let path = entry.path();
            if path.is_dir() {
                if depth < MAX_DEPTH {
                    subdirectories.push((path, depth.saturating_add(1)));
                }
                continue;
            }
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let name = name.to_lowercase();
            let is_font = name
                .rsplit_once('.')
                .is_some_and(|(_, extension)| FONT_EXTENSIONS.contains(&extension));
            if is_font {
                files.entry(name).or_insert(path);
            }
        }
        subdirectories.sort();
        pending.extend(subdirectories.into_iter().rev());
    }
    files
}

fn load(path: &Path) -> Option<FallbackFont> {
    let size = fs::metadata(path).ok()?.len();
    if size > MAX_FONT_BYTES {
        log::debug!("{} is too large to load as a fallback font", path.display());
        return None;
    }
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            log::debug!("{} could not be read: {error}", path.display());
            return None;
        }
    };
    if !is_loadable(&bytes) {
        log::debug!("{} is not a font egui can load", path.display());
        return None;
    }
    let name = path.file_name()?.to_string_lossy().to_lowercase();
    log::info!("{} is used for text in other scripts", path.display());
    Some(FallbackFont {
        key: format!("{KEY_PREFIX}{name}"),
        data: Arc::new(FontData::from_owned(bytes)),
    })
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    let end = at.checked_add(2)?;
    Some(u16::from_be_bytes(bytes.get(at..end)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    let end = at.checked_add(4)?;
    Some(u32::from_be_bytes(bytes.get(at..end)?.try_into().ok()?))
}

fn first_table_directory(bytes: &[u8]) -> Option<usize> {
    if bytes.get(0..4) != Some(COLLECTION_TAG.as_slice()) {
        return Some(0);
    }
    let major = u16_at(bytes, 4)?;
    let count = usize::try_from(u32_at(bytes, 8)?).ok()?;
    let offsets = count.checked_mul(4)?;
    let signature = if major >= 2 {
        COLLECTION_SIGNATURE_FIELDS
    } else {
        0
    };
    let header_end = COLLECTION_HEADER
        .checked_add(offsets)?
        .checked_add(signature)?;
    if count == 0 || bytes.len() < header_end {
        return None;
    }
    usize::try_from(u32_at(bytes, COLLECTION_HEADER)?).ok()
}

fn is_loadable(bytes: &[u8]) -> bool {
    let Some(directory) = first_table_directory(bytes) else {
        return false;
    };
    let Some(version) = u32_at(bytes, directory) else {
        return false;
    };
    let Some(tables) = u16_at(bytes, directory.saturating_add(4)) else {
        return false;
    };
    let end = usize::from(tables)
        .checked_mul(TABLE_RECORD)
        .and_then(|records| records.checked_add(TABLE_DIRECTORY_HEADER))
        .and_then(|length| length.checked_add(directory));
    OUTLINE_VERSIONS.contains(&version) && end.is_some_and(|end| end <= bytes.len())
}

#[cfg(unix)]
fn font_directories() -> Vec<PathBuf> {
    let absolute = |path: PathBuf| path.is_absolute().then_some(path);
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .and_then(absolute);
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .and_then(absolute)
        .or_else(|| home.as_ref().map(|home| home.join(".local").join("share")));
    let data_dirs: Vec<PathBuf> = std::env::var_os("XDG_DATA_DIRS")
        .filter(|dirs| !dirs.is_empty())
        .map(|dirs| std::env::split_paths(&dirs).filter_map(absolute).collect())
        .unwrap_or_else(|| {
            vec![
                PathBuf::from("/usr/local/share"),
                PathBuf::from("/usr/share"),
            ]
        });
    let mut directories: Vec<PathBuf> = data_home
        .into_iter()
        .chain(data_dirs)
        .map(|directory| directory.join("fonts"))
        .chain(home.map(|home| home.join(".fonts")))
        .chain([PathBuf::from("/usr/share/fonts")])
        .collect();
    let mut seen = Vec::new();
    directories.retain(|directory| {
        let fresh = !seen.contains(directory);
        seen.push(directory.clone());
        fresh
    });
    directories
}

#[cfg(windows)]
fn font_directories() -> Vec<PathBuf> {
    let system = std::env::var_os("WINDIR")
        .or_else(|| std::env::var_os("SystemRoot"))
        .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from)
        .join("Fonts");
    let user = std::env::var_os("LOCALAPPDATA").map(|local| {
        PathBuf::from(local)
            .join("Microsoft")
            .join("Windows")
            .join("Fonts")
    });
    std::iter::once(system).chain(user).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory(version: &[u8; 4], tables: u16) -> Vec<u8> {
        let mut bytes = version.to_vec();
        bytes.extend(tables.to_be_bytes());
        bytes.extend([0; 6]);
        bytes.extend(vec![0; usize::from(tables) * TABLE_RECORD]);
        bytes
    }

    #[test]
    fn only_whole_font_headers_are_loaded() {
        let truetype = directory(&[0, 1, 0, 0], 3);
        let mut truncated = truetype.clone();
        truncated.pop();
        let mut collection = b"ttcf".to_vec();
        collection.extend([0, 1, 0, 0]);
        collection.extend(1_u32.to_be_bytes());
        collection.extend(16_u32.to_be_bytes());
        collection.extend(directory(b"OTTO", 2));
        let mut out_of_range = b"ttcf".to_vec();
        out_of_range.extend([0, 1, 0, 0]);
        out_of_range.extend(1_u32.to_be_bytes());
        out_of_range.extend(4096_u32.to_be_bytes());

        assert!(is_loadable(&truetype));
        assert!(is_loadable(&collection));
        assert!(is_loadable(crate::fonts::INTER));
        assert!(!is_loadable(&truncated));
        assert!(!is_loadable(&directory(b"wOFF", 0)));
        assert!(!is_loadable(&out_of_range));
        assert!(!is_loadable(&[]));
    }

    #[test]
    fn a_variable_font_stands_in_for_its_regular_file() {
        let files: BTreeMap<String, PathBuf> = [
            ("notosansarabic[wdth,wght].ttf", "/fonts/arabic.ttf"),
            ("notosanshebrew-regular.ttf", "/fonts/hebrew.ttf"),
        ]
        .into_iter()
        .map(|(name, path)| (name.to_owned(), PathBuf::from(path)))
        .collect();

        assert_eq!(
            lookup(&files, "NotoSansHebrew-Regular.ttf"),
            Some(Path::new("/fonts/hebrew.ttf"))
        );
        assert_eq!(
            lookup(&files, "NotoSansArabic-Regular.ttf"),
            Some(Path::new("/fonts/arabic.ttf"))
        );
        assert_eq!(lookup(&files, "NotoSansThai-Regular.ttf"), None);
    }

    #[test]
    fn found_fonts_load_into_egui_and_render_other_scripts() {
        let found = find();
        let samples = [
            ("cjk", "漢字かな한글"),
            ("arabic", "عربي"),
            ("devanagari", "देवनागरी"),
            ("thai", "ภาษาไทย"),
        ];
        let context = egui::Context::default();
        context.set_fonts(crate::fonts::definitions_with(&found));
        let mut output = context.run_ui(egui::RawInput::default(), |_| {});
        output.textures_delta.clear();
        let font = egui::FontId::proportional(14.0);
        let latin = context.fonts_mut(|fonts| fonts.has_glyphs(&font, "Ag0"));
        let missing: Vec<&str> = samples
            .iter()
            .filter(|(part, _)| found.iter().any(|script| script.key.contains(part)))
            .filter(|(_, text)| !context.fonts_mut(|fonts| fonts.has_glyphs(&font, text)))
            .map(|(part, _)| *part)
            .collect();

        assert!(latin);
        assert_eq!(missing, Vec::<&str>::new());
    }
}
