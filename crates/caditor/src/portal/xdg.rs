use std::{
    collections::BTreeMap,
    ffi::OsString,
    io,
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

use zbus::{
    blocking::{Connection, Proxy},
    fdo,
    zvariant::{OwnedObjectPath, OwnedValue, Value},
};

use super::{ANY_EXTENSION, DialogError, FileRequest, Filter, Mode};

const DESKTOP: &str = "org.freedesktop.portal.Desktop";
const DESKTOP_PATH: &str = "/org/freedesktop/portal/desktop";
const FILE_CHOOSER: &str = "org.freedesktop.portal.FileChooser";
const REQUEST: &str = "org.freedesktop.portal.Request";
const REQUEST_PATH: &str = "/org/freedesktop/portal/desktop/request";
const RESPONSE: &str = "Response";
const SUCCESS: u32 = 0;
const FILE_SCHEME: &str = "file://";
const LOCAL_HOST: &str = "localhost";
const ZENITY: &str = "zenity";
const ZENITY_CANCELLED: i32 = 1;
const MISSING: [&str; 4] = [
    "org.freedesktop.DBus.Error.ServiceUnknown",
    "org.freedesktop.DBus.Error.UnknownInterface",
    "org.freedesktop.DBus.Error.UnknownMethod",
    "org.freedesktop.DBus.Error.UnknownObject",
];

static NEXT_TOKEN: AtomicU64 = AtomicU64::new(0);
static OWNER: OnceLock<ParentWindow> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParentWindow {
    X11(u64),
    Wayland(String),
}

impl ParentWindow {
    fn identifier(&self) -> String {
        match self {
            Self::X11(window) => format!("x11:{window:x}"),
            Self::Wayland(identifier) => identifier.clone(),
        }
    }
}

pub fn own_dialogs(parent: ParentWindow) {
    if OWNER.set(parent).is_err() {
        log::debug!("file dialogs already belong to the first window");
    }
}

fn parent_identifier() -> String {
    OWNER
        .get()
        .map(|parent| parent.identifier())
        .unwrap_or_default()
}

fn patterns(filter: &Filter) -> Vec<String> {
    filter
        .extensions
        .iter()
        .map(|extension| match extension.as_str() {
            ANY_EXTENSION => ANY_EXTENSION.to_owned(),
            extension => format!("*.{}", either_case(extension)),
        })
        .collect()
}

fn either_case(extension: &str) -> String {
    extension
        .chars()
        .map(|character| {
            if character.is_ascii_alphabetic() {
                format!(
                    "[{}{}]",
                    character.to_ascii_lowercase(),
                    character.to_ascii_uppercase()
                )
            } else {
                character.to_string()
            }
        })
        .collect()
}

pub fn choose(request: &FileRequest) -> Result<Option<PathBuf>, DialogError> {
    match through_portal(request) {
        Err(DialogError::NoPortal(error) | DialogError::NoSessionBus(error)) => {
            log::warn!("no desktop portal for a file dialog ({error}); trying zenity");
            through_zenity(request).unwrap_or(Err(DialogError::NoPortal(error)))
        }
        chosen => chosen,
    }
}

fn through_portal(request: &FileRequest) -> Result<Option<PathBuf>, DialogError> {
    let connection = Connection::session().map_err(DialogError::NoSessionBus)?;
    let token = format!(
        "caditor_{}_{}",
        std::process::id(),
        NEXT_TOKEN.fetch_add(1, Ordering::Relaxed)
    );
    let sender = connection
        .unique_name()
        .map(|name| name.as_str().trim_start_matches(':').replace('.', "_"))
        .unwrap_or_default();
    let expected = format!("{REQUEST_PATH}/{sender}/{token}");
    let responses = Proxy::new(&connection, DESKTOP, expected.as_str(), REQUEST)
        .and_then(|proxy| proxy.receive_signal(RESPONSE))
        .map_err(DialogError::Portal)?;
    let chooser = Proxy::new(&connection, DESKTOP, DESKTOP_PATH, FILE_CHOOSER)
        .map_err(DialogError::Portal)?;

    let method = match request.mode {
        Mode::Open => "OpenFile",
        Mode::Save => "SaveFile",
    };
    let options = portal_options(request, &token);
    let parent = parent_identifier();
    let handle: OwnedObjectPath = chooser
        .call(method, &(parent.as_str(), request.title.as_str(), options))
        .map_err(classify)?;
    let mut responses = if handle.as_str() == expected {
        responses
    } else {
        Proxy::new(&connection, DESKTOP, handle.as_str(), REQUEST)
            .and_then(|proxy| proxy.receive_signal(RESPONSE))
            .map_err(DialogError::Portal)?
    };
    let Some(message) = responses.next() else {
        return Ok(None);
    };
    let (code, results): (u32, BTreeMap<String, OwnedValue>) =
        message.body().deserialize().map_err(DialogError::Portal)?;
    if code != SUCCESS {
        return Ok(None);
    }
    let uris = results
        .get("uris")
        .map(|uris| Vec::<String>::try_from(uris.clone()))
        .transpose()
        .map_err(|error| DialogError::Portal(error.into()))?
        .unwrap_or_default();
    let Some(uri) = uris.into_iter().next() else {
        return Ok(None);
    };
    local_path(&uri)
        .map(Some)
        .ok_or(DialogError::NotALocalFile(uri))
}

fn portal_options<'a>(
    request: &'a FileRequest,
    token: &'a str,
) -> BTreeMap<&'static str, Value<'a>> {
    let filters: Vec<(String, Vec<(u32, String)>)> = request
        .filters
        .iter()
        .map(|filter| {
            let globs = patterns(filter)
                .into_iter()
                .map(|pattern| (0, pattern))
                .collect();
            (filter.name.clone(), globs)
        })
        .collect();
    let mut options: BTreeMap<&'static str, Value<'a>> = BTreeMap::new();
    options.insert("handle_token", Value::from(token));
    options.insert("modal", Value::from(true));
    if let Some(first) = filters.first().cloned() {
        options.insert("current_filter", Value::from(first));
    }
    options.insert("filters", Value::from(filters));
    if let Some(name) = &request.file_name {
        options.insert("current_name", Value::from(name.as_str()));
    }
    if let Some(directory) = &request.directory {
        let mut bytes = directory.as_os_str().as_bytes().to_vec();
        bytes.push(0);
        options.insert("current_folder", Value::from(bytes));
    }
    options
}

fn classify(error: zbus::Error) -> DialogError {
    let missing = match &error {
        zbus::Error::MethodError(name, _, _) => MISSING.contains(&name.as_str()),
        zbus::Error::FDO(fdo) => matches!(
            **fdo,
            fdo::Error::ServiceUnknown(_)
                | fdo::Error::UnknownInterface(_)
                | fdo::Error::UnknownMethod(_)
                | fdo::Error::UnknownObject(_)
        ),
        _ => false,
    };
    if missing {
        DialogError::NoPortal(error)
    } else {
        DialogError::Portal(error)
    }
}

fn local_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix(FILE_SCHEME)?;
    let path = rest.strip_prefix(LOCAL_HOST).unwrap_or(rest);
    if !path.starts_with('/') {
        return None;
    }
    let mut bytes = Vec::with_capacity(path.len());
    let mut rest = path.as_bytes();
    while let Some((&byte, after)) = rest.split_first() {
        if byte == b'%' {
            let hex = after.get(..2)?;
            let text = std::str::from_utf8(hex).ok()?;
            bytes.push(u8::from_str_radix(text, 16).ok()?);
            rest = after.get(2..)?;
        } else {
            bytes.push(byte);
            rest = after;
        }
    }
    Some(PathBuf::from(OsString::from_vec(bytes)))
}

fn through_zenity(request: &FileRequest) -> Option<Result<Option<PathBuf>, DialogError>> {
    let mut command = Command::new(ZENITY);
    command
        .arg("--file-selection")
        .arg(format!("--title={}", request.title));
    if request.mode == Mode::Save {
        command.arg("--save").arg("--confirm-overwrite");
    }
    let start = zenity_start(request.directory.as_deref(), request.file_name.as_deref());
    if let Some(start) = start {
        let mut argument = OsString::from("--filename=");
        argument.push(start);
        command.arg(argument);
    }
    for filter in &request.filters {
        command.arg(format!(
            "--file-filter={} | {}",
            filter.name,
            patterns(filter).join(" ")
        ));
    }
    let output = match command.stdin(Stdio::null()).stderr(Stdio::null()).output() {
        Ok(output) => output,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
        Err(error) => return Some(Err(DialogError::Zenity(error))),
    };
    let chosen = match output.status.code() {
        Some(0) => {
            let mut path = output.stdout;
            while path.last().is_some_and(|byte| *byte == b'\n') {
                path.pop();
            }
            Ok((!path.is_empty()).then(|| PathBuf::from(OsString::from_vec(path))))
        }
        Some(ZENITY_CANCELLED) | None => Ok(None),
        Some(code) => Err(DialogError::ZenityFailed(code)),
    };
    Some(chosen)
}

fn zenity_start(directory: Option<&Path>, file_name: Option<&str>) -> Option<PathBuf> {
    match (directory, file_name) {
        (Some(directory), Some(name)) => Some(directory.join(name)),
        (Some(directory), None) => {
            let mut start = directory.as_os_str().to_os_string();
            start.push("/");
            Some(PathBuf::from(start))
        }
        (None, Some(name)) => Some(PathBuf::from(name)),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_uris_become_paths_with_their_escapes_decoded() {
        assert_eq!(
            local_path("file:///home/someone/a%20plate%2B1.caditor"),
            Some(PathBuf::from("/home/someone/a plate+1.caditor"))
        );
        assert_eq!(
            local_path("file://localhost/tmp/x.step"),
            Some(PathBuf::from("/tmp/x.step"))
        );
        assert_eq!(
            local_path("file:///tmp/%FF.dxf"),
            Some(PathBuf::from(OsString::from_vec(b"/tmp/\xff.dxf".to_vec())))
        );
    }

    #[test]
    fn remote_or_malformed_uris_are_not_local_paths() {
        assert_eq!(local_path("https://example.com/a.caditor"), None);
        assert_eq!(local_path("file://server/share/a.caditor"), None);
        assert_eq!(local_path("file:///tmp/%4"), None);
        assert_eq!(local_path("file:///tmp/%zz"), None);
    }

    #[test]
    fn the_options_carry_the_token_filters_name_and_folder() {
        let request = FileRequest {
            mode: Mode::Save,
            title: "Save model".to_owned(),
            directory: Some(PathBuf::from("/models")),
            file_name: Some("plate.caditor".to_owned()),
            filters: vec![Filter::new("caditor model", &["caditor"])],
        };

        let options = portal_options(&request, "caditor_1_2");

        assert_eq!(options["handle_token"], Value::from("caditor_1_2"));
        assert_eq!(options["current_name"], Value::from("plate.caditor"));
        assert_eq!(
            options["current_folder"],
            Value::from(b"/models\0".to_vec())
        );
        assert_eq!(
            options["filters"],
            Value::from(vec![(
                "caditor model".to_owned(),
                vec![(0_u32, "*.[cC][aA][dD][iI][tT][oO][rR]".to_owned())]
            )])
        );
    }

    #[test]
    fn filters_match_either_case_and_all_files_match_anything() {
        assert_eq!(
            patterns(&Filter::new("Drawings and models", &["dxf", "3mf", "stp"])),
            ["*.[dD][xX][fF]", "*.3[mM][fF]", "*.[sS][tT][pP]"]
        );
        assert_eq!(patterns(&Filter::any()), ["*"]);
    }

    #[test]
    fn an_x11_parent_is_named_by_its_window_id_in_hexadecimal() {
        assert_eq!(ParentWindow::X11(0x3a0_0007).identifier(), "x11:3a00007");
    }

    #[test]
    fn a_wayland_parent_is_named_by_its_exported_handle() {
        let parent = ParentWindow::Wayland("wayland:d5c30dbd".to_owned());

        assert_eq!(parent.identifier(), "wayland:d5c30dbd");
    }

    #[test]
    fn a_missing_portal_tells_what_to_install() {
        let missing = classify(zbus::Error::FDO(Box::new(fdo::Error::ServiceUnknown(
            "org.freedesktop.portal.Desktop".to_owned(),
        ))));

        assert!(matches!(missing, DialogError::NoPortal(_)));
        assert!(missing.notice().contains("xdg-desktop-portal"));
    }
}
