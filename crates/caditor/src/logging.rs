use std::{
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use caditor_file::SessionLog;
use caditor_render::RenderError;
use env_logger::{Builder, Env, Target};

use crate::about;

const DEFAULT_FILTER: &str = "info";
#[cfg(unix)]
const NOTIFIER: &str = "notify-send";
#[cfg(unix)]
const GRAPHICS_ADVICE: &str = "\n\nThe graphics driver may be at fault. Starting caditor with \
                               WGPU_BACKEND=gl set uses OpenGL instead of Vulkan and often \
                               helps: run “WGPU_BACKEND=gl caditor” in a terminal, or add it to \
                               the start of the menu entry's command.";
#[cfg(windows)]
const GRAPHICS_ADVICE: &str = "\n\nThe graphics driver may be at fault: install the latest one \
                               from the maker of the graphics card. Setting the user environment \
                               variable WGPU_BACKEND=vulkan (or WGPU_BACKEND=gl) makes caditor \
                               draw through Vulkan or OpenGL instead of Direct3D 12 and often \
                               helps.";

pub struct Logging {
    log: Option<Arc<SessionLog>>,
}

impl Logging {
    pub fn start(state_dir: Option<&Path>) -> Self {
        let created = state_dir.map(SessionLog::create);
        let (log, problem) = match created {
            Some(Ok(log)) => (Some(Arc::new(log)), None),
            Some(Err(error)) => (None, Some(error)),
            None => (None, None),
        };
        Builder::from_env(Env::default().default_filter_or(DEFAULT_FILTER))
            .target(Target::Pipe(Box::new(Tee {
                stderr: io::stderr(),
                log: log.clone(),
            })))
            .init();
        if let Some(problem) = problem {
            log::warn!("could not start a log file: {problem}");
        }
        if let Some(log) = &log {
            log::info!(
                "{} is logging to {}",
                about::version_line(),
                log.path().display()
            );
        }
        Self { log }
    }

    pub fn path(&self) -> Option<&Path> {
        self.log.as_deref().map(SessionLog::path)
    }

    pub fn ending(&self) -> Option<Arc<SessionLog>> {
        self.log.clone()
    }

    pub fn earlier_unexpected_end(&self, state_dir: &Path) -> Option<PathBuf> {
        let own = self.path()?;
        let stopped = caditor_file::ended_unexpectedly(state_dir, own);
        for log in &stopped {
            if let Err(error) = caditor_file::mark_reported(log) {
                log::warn!("could not mark {} as reported: {error}", log.display());
            }
        }
        caditor_file::prune_logs(state_dir, own);
        stopped.into_iter().next()
    }

    pub fn end(&self) {
        if let Some(log) = &self.log {
            log.end();
        }
    }
}

struct Tee {
    stderr: io::Stderr,
    log: Option<Arc<SessionLog>>,
}

impl Write for Tee {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let to_stderr = self.stderr.write_all(bytes);
        let to_log = self.log.as_ref().map_or(Ok(()), |log| log.write(bytes));
        to_stderr.or(to_log).map(|()| bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stderr.flush()
    }
}

pub fn unexpected_end_notice(log: &Path) -> String {
    format!(
        "caditor stopped unexpectedly last time. What it logged then is kept in {}.",
        log.display()
    )
}

pub fn failure_text(error: &anyhow::Error, log: Option<&Path>) -> String {
    let mut text = format!("caditor had to stop: {error:#}.");
    if error.chain().any(|cause| cause.is::<RenderError>()) {
        text.push_str(GRAPHICS_ADVICE);
    }
    if let Some(log) = log {
        text.push_str(&format!("\n\nWhat caditor logged is in {}.", log.display()));
    }
    text
}

#[cfg(windows)]
pub fn show_failure(text: &str) {
    if io::stderr().is_terminal() {
        return;
    }
    if !caditor_windows::show_error(about::NAME, text) {
        log::warn!(
            "the failure could not be shown: {}",
            io::Error::last_os_error()
        );
    }
}

#[cfg(unix)]
pub fn show_failure(text: &str) {
    use std::process::{Command, Stdio};

    if io::stderr().is_terminal() {
        return;
    }
    let attempts: [(&str, Vec<&str>); 3] = [
        (
            "zenity",
            vec![
                "--error",
                "--no-markup",
                "--title",
                about::NAME,
                "--text",
                text,
            ],
        ),
        ("kdialog", vec!["--title", about::NAME, "--error", text]),
        (NOTIFIER, vec!["--urgency=critical", about::NAME, text]),
    ];
    let display = ["WAYLAND_DISPLAY", "DISPLAY"]
        .iter()
        .any(|variable| std::env::var_os(variable).is_some_and(|value| !value.is_empty()));
    let usable = |program: &str| display || program == NOTIFIER;
    let shown =
        attempts
            .iter()
            .filter(|(program, _)| usable(program))
            .any(|(program, arguments)| {
                Command::new(program)
                    .args(arguments)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .is_ok_and(|status| status.success())
            });
    if !shown {
        log::warn!("no dialog program (zenity, kdialog or notify-send) could show the failure");
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Context;

    use super::*;

    #[test]
    fn a_renderer_failure_names_the_opengl_workaround_and_the_log() {
        let failed: anyhow::Result<()> =
            Err(RenderError::NoAdapter).context("could not start the renderer");
        let error = failed.unwrap_err();

        let text = failure_text(&error, Some(Path::new("/state/logs/caditor-1-2.log")));

        assert!(text.starts_with("caditor had to stop: could not start the renderer: "));
        assert!(text.contains("WGPU_BACKEND=gl"));
        assert!(text.contains(GRAPHICS_ADVICE));
        assert!(text.ends_with("What caditor logged is in /state/logs/caditor-1-2.log."));
    }

    #[test]
    fn other_failures_leave_the_graphics_advice_out() {
        let error = anyhow::anyhow!("no display");

        let text = failure_text(&error, None);

        assert_eq!(text, "caditor had to stop: no display.");
    }
}
