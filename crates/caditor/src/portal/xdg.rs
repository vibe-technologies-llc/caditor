use std::{
    collections::BTreeMap,
    ffi::OsString,
    io,
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        process::ExitStatusExt,
    },
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
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

use super::{ANY_EXTENSION, DialogError, FileRequest, Filter, Mode, SettingError};

const DESKTOP: &str = "org.freedesktop.portal.Desktop";
const DESKTOP_PATH: &str = "/org/freedesktop/portal/desktop";
const FILE_CHOOSER: &str = "org.freedesktop.portal.FileChooser";
const REQUEST: &str = "org.freedesktop.portal.Request";
const SETTINGS: &str = "org.freedesktop.portal.Settings";
const READ_ONE: &str = "ReadOne";
const SETTING_CHANGED: &str = "SettingChanged";
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MotionKey {
    ReducedMotion,
    GnomeAnimations,
    KdeAnimationFactor,
}

impl MotionKey {
    const ALL: [Self; 3] = [
        Self::ReducedMotion,
        Self::GnomeAnimations,
        Self::KdeAnimationFactor,
    ];

    fn setting(self) -> (&'static str, &'static str) {
        match self {
            Self::ReducedMotion => ("org.freedesktop.appearance", "reduced-motion"),
            Self::GnomeAnimations => ("org.gnome.desktop.interface", "enable-animations"),
            Self::KdeAnimationFactor => ("org.kde.kdeglobals.KDE", "AnimationDurationFactor"),
        }
    }

    fn named(namespace: &str, key: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.setting() == (namespace, key))
    }

    fn asks_to_reduce(self, value: &Value) -> Option<bool> {
        let value = unwrapped(value);
        match self {
            Self::ReducedMotion => Some(matches!(value, Value::U32(1))),
            Self::GnomeAnimations => match value {
                Value::Bool(enabled) => Some(!enabled),
                _ => None,
            },
            Self::KdeAnimationFactor => number(value).map(|factor| factor <= 0.0),
        }
    }
}

fn unwrapped<'a, 'v>(value: &'a Value<'v>) -> &'a Value<'v> {
    match value {
        Value::Value(inner) => unwrapped(inner),
        value => value,
    }
}

fn number(value: &Value) -> Option<f64> {
    match value {
        Value::U8(number) => Some(f64::from(*number)),
        Value::I16(number) => Some(f64::from(*number)),
        Value::U16(number) => Some(f64::from(*number)),
        Value::I32(number) => Some(f64::from(*number)),
        Value::U32(number) => Some(f64::from(*number)),
        Value::F64(number) => Some(*number),
        Value::Str(text) => text.as_str().trim().parse().ok(),
        _ => None,
    }
}

fn any_asks_to_reduce(asks: impl IntoIterator<Item = Option<bool>>) -> bool {
    asks.into_iter().flatten().any(|reduce| reduce)
}

fn settings_proxy(connection: &Connection) -> Result<Proxy<'static>, SettingError> {
    Proxy::new(connection, DESKTOP, DESKTOP_PATH, SETTINGS).map_err(SettingError::Unavailable)
}

fn read_reduces_motion(settings: &Proxy) -> bool {
    any_asks_to_reduce(MotionKey::ALL.map(|key| {
        let (namespace, name) = key.setting();
        let value: OwnedValue = settings.call(READ_ONE, &(namespace, name)).ok()?;
        key.asks_to_reduce(&value)
    }))
}

pub fn reduces_motion() -> Result<bool, SettingError> {
    let connection = Connection::session().map_err(SettingError::NoSessionBus)?;
    Ok(read_reduces_motion(&settings_proxy(&connection)?))
}

pub fn watch_motion(mut changed: impl FnMut(bool)) -> Result<(), SettingError> {
    let connection = Connection::session().map_err(SettingError::NoSessionBus)?;
    let settings = settings_proxy(&connection)?;
    let signals = settings
        .receive_signal(SETTING_CHANGED)
        .map_err(SettingError::Unavailable)?;

    changed(read_reduces_motion(&settings));
    for message in signals {
        let concerns_motion = message
            .body()
            .deserialize::<(String, String, OwnedValue)>()
            .is_ok_and(|(namespace, key, _)| MotionKey::named(&namespace, &key).is_some());
        if concerns_motion {
            changed(read_reduces_motion(&settings));
        }
    }
    Ok(())
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
    let response = responses
        .next()
        .map(|message| message.body().deserialize())
        .transpose()
        .map_err(DialogError::Portal)?;
    portal_choice(response)
}

fn portal_choice(
    response: Option<(u32, BTreeMap<String, OwnedValue>)>,
) -> Result<Option<PathBuf>, DialogError> {
    let (code, results) = response.ok_or(DialogError::PortalClosed)?;
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
    Some(zenity_choice(output.status, output.stdout))
}

fn zenity_choice(status: ExitStatus, mut stdout: Vec<u8>) -> Result<Option<PathBuf>, DialogError> {
    match status.code() {
        Some(0) => {
            while stdout.last().is_some_and(|byte| *byte == b'\n') {
                stdout.pop();
            }
            Ok((!stdout.is_empty()).then(|| PathBuf::from(OsString::from_vec(stdout))))
        }
        Some(ZENITY_CANCELLED) => Ok(None),
        Some(code) => Err(DialogError::ZenityFailed(code)),
        None => Err(DialogError::ZenityKilled(status.signal())),
    }
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
    fn the_portal_key_asks_to_reduce_only_for_one() {
        let key = MotionKey::ReducedMotion;

        assert_eq!(key.asks_to_reduce(&Value::from(1_u32)), Some(true));
        assert_eq!(key.asks_to_reduce(&Value::from(0_u32)), Some(false));
        assert_eq!(key.asks_to_reduce(&Value::from(7_u32)), Some(false));
    }

    #[test]
    fn gnome_asks_to_reduce_when_animations_are_off() {
        let key = MotionKey::GnomeAnimations;

        assert_eq!(key.asks_to_reduce(&Value::from(false)), Some(true));
        assert_eq!(key.asks_to_reduce(&Value::from(true)), Some(false));
        assert_eq!(key.asks_to_reduce(&Value::from("true")), None);
    }

    #[test]
    fn plasma_asks_to_reduce_at_factor_zero_as_a_number_or_text() {
        let key = MotionKey::KdeAnimationFactor;

        assert_eq!(key.asks_to_reduce(&Value::from(0.0_f64)), Some(true));
        assert_eq!(key.asks_to_reduce(&Value::from("0")), Some(true));
        assert_eq!(key.asks_to_reduce(&Value::from(" 0.0 ")), Some(true));
        assert_eq!(key.asks_to_reduce(&Value::from(0.5_f64)), Some(false));
        assert_eq!(key.asks_to_reduce(&Value::from("2")), Some(false));
        assert_eq!(key.asks_to_reduce(&Value::from("fast")), None);
    }

    #[test]
    fn a_value_wrapped_in_a_variant_is_read_through() {
        let wrapped = Value::Value(Box::new(Value::from(false)));

        assert_eq!(
            MotionKey::GnomeAnimations.asks_to_reduce(&wrapped),
            Some(true)
        );
    }

    #[test]
    fn any_key_asking_to_reduce_wins_and_missing_keys_do_not_count() {
        assert!(any_asks_to_reduce([Some(false), None, Some(true)]));
        assert!(any_asks_to_reduce([None, Some(true), None]));
        assert!(!any_asks_to_reduce([Some(false), Some(false), None]));
        assert!(!any_asks_to_reduce([None, None, None]));
    }

    #[test]
    fn only_the_three_motion_settings_are_followed_in_a_change() {
        assert_eq!(
            MotionKey::named("org.freedesktop.appearance", "reduced-motion"),
            Some(MotionKey::ReducedMotion)
        );
        assert_eq!(
            MotionKey::named("org.gnome.desktop.interface", "enable-animations"),
            Some(MotionKey::GnomeAnimations)
        );
        assert_eq!(
            MotionKey::named("org.kde.kdeglobals.KDE", "AnimationDurationFactor"),
            Some(MotionKey::KdeAnimationFactor)
        );
        assert_eq!(
            MotionKey::named("org.freedesktop.appearance", "color-scheme"),
            None
        );
    }

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

    fn status_of_exit(code: i32) -> ExitStatus {
        ExitStatus::from_raw(code << 8)
    }

    fn status_of_signal(signal: i32) -> ExitStatus {
        ExitStatus::from_raw(signal)
    }

    fn uris_response(code: u32, uris: &[&str]) -> (u32, BTreeMap<String, OwnedValue>) {
        let uris = Value::from(uris.iter().map(|uri| (*uri).to_owned()).collect::<Vec<_>>());
        let results = BTreeMap::from([("uris".to_owned(), OwnedValue::try_from(uris).unwrap())]);
        (code, results)
    }

    #[test]
    fn a_portal_that_closes_without_a_response_is_an_error_not_a_cancel() {
        let closed = portal_choice(None);

        assert!(matches!(closed, Err(DialogError::PortalClosed)));
        assert!(closed.unwrap_err().notice().contains("Try again"));
    }

    #[test]
    fn a_portal_answers_with_a_path_or_a_cancel() {
        assert_eq!(
            portal_choice(Some(uris_response(0, &["file:///tmp/a.caditor"]))).unwrap(),
            Some(PathBuf::from("/tmp/a.caditor"))
        );
        assert_eq!(portal_choice(Some(uris_response(1, &[]))).unwrap(), None);
        assert_eq!(portal_choice(Some(uris_response(0, &[]))).unwrap(), None);
    }

    #[test]
    fn zenity_exiting_with_status_one_is_a_cancel() {
        assert_eq!(zenity_choice(status_of_exit(1), Vec::new()).unwrap(), None);
    }

    #[test]
    fn zenity_printing_a_path_chooses_it() {
        assert_eq!(
            zenity_choice(status_of_exit(0), b"/tmp/a.caditor\n".to_vec()).unwrap(),
            Some(PathBuf::from("/tmp/a.caditor"))
        );
    }

    #[test]
    fn zenity_exiting_with_another_status_is_an_error() {
        assert!(matches!(
            zenity_choice(status_of_exit(5), Vec::new()),
            Err(DialogError::ZenityFailed(5))
        ));
    }

    #[test]
    fn zenity_killed_by_a_signal_is_an_error_not_a_cancel() {
        let killed = zenity_choice(status_of_signal(9), Vec::new());

        assert!(matches!(killed, Err(DialogError::ZenityKilled(Some(9)))));
        assert!(killed.unwrap_err().notice().contains("Try again"));
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
