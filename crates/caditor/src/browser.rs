use std::{
    ffi::OsStr,
    io,
    process::{Command, Stdio},
    thread,
};

#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("Only secure web addresses are opened in the browser.")]
    NotSecure,
    #[error("caditor could not start the web browser: {0}.")]
    Unavailable(io::ErrorKind),
}

pub fn open(address: &str) -> Result<(), OpenError> {
    let secure = address.starts_with("https://") && !address.chars().any(char::is_whitespace);

    if !secure {
        return Err(OpenError::NotSecure);
    }
    launch(address).map_err(|error| OpenError::Unavailable(error.kind()))
}

fn start(program: &str, leading: &[&str], address: &OsStr) -> io::Result<()> {
    let mut command = Command::new(program);

    command.args(leading).arg(address);
    detach(command)
}

pub(crate) fn detach(mut command: Command) -> io::Result<()> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    let reaper = thread::Builder::new()
        .name("browser".to_owned())
        .spawn(move || {
            if let Err(error) = child.wait() {
                log::debug!("the browser launcher could not be waited for: {error}");
            }
        });

    if let Err(error) = reaper {
        log::debug!("the browser launcher is left to finish by itself: {error}");
    }
    Ok(())
}

#[cfg(unix)]
pub(crate) fn launch(address: impl AsRef<OsStr>) -> io::Result<()> {
    let address = address.as_ref();
    start("xdg-open", &[], address).or_else(|_| start("gio", &["open"], address))
}

#[cfg(windows)]
pub(crate) fn launch(address: impl AsRef<OsStr>) -> io::Result<()> {
    start(
        "rundll32",
        &["url.dll,FileProtocolHandler"],
        address.as_ref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_secure_addresses_without_spaces_are_opened() {
        for refused in [
            "http://example.com",
            "file:///etc/passwd",
            "--help",
            "https://example.com/a b",
            "",
        ] {
            assert!(
                matches!(open(refused), Err(OpenError::NotSecure)),
                "{refused:?}"
            );
        }
    }
}
