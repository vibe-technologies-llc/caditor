use std::{
    env,
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Command, ExitStatus, Stdio},
    sync::Arc,
    thread,
    time::Duration,
};

use caditor::crash::{self, PanicFlush};
use caditor_document::Document;
use caditor_expression::Expression;
use caditor_file::{JournalEntry, SessionLog, Start, Storage, StorageConfig};
use parking_lot::Mutex;

const MODE: &str = "CADITOR_CRASH_CHILD";
const DIR: &str = "CADITOR_CRASH_DIR";
const CHILD_TEST: &str = "crash_child";
const WRITTEN: &str = "journal written";
const CHANGES: usize = 100;
const FLUSH_TIMEOUT: Duration = Duration::from_secs(10);
const PANIC_EXIT_CODE: i32 = 101;
const STATE: &str = "state";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ending {
    Panic,
    Terminate,
    Kill,
}

impl Ending {
    const ALL: [Self; 3] = [Self::Panic, Self::Terminate, Self::Kill];

    fn name(self) -> &'static str {
        match self {
            Self::Panic => "panic",
            Self::Terminate => "terminate",
            Self::Kill => "kill",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|ending| ending.name() == name)
    }
}

#[test]
#[ignore = "runs only as the process the crash tests spawn and stop"]
fn crash_child() {
    let (Some(ending), Some(dir)) = (
        env::var(MODE).ok().as_deref().and_then(Ending::from_name),
        env::var_os(DIR),
    ) else {
        return;
    };

    let storage = Storage::spawn(
        StorageConfig {
            recovery_dir: Some(dir.clone().into()),
            ..StorageConfig::default()
        },
        Start {
            file: None,
            on_disk: None,
            loaded_with_problems: false,
            base: Document::default(),
            folded: 0,
            entries: Vec::new(),
            replaces: None,
            after: None,
        },
        || {},
    )
    .unwrap();
    let panic_flush: PanicFlush = Arc::new(Mutex::new(Some(storage.flusher())));
    let log = SessionLog::create(&Path::new(&dir).join(STATE)).unwrap();
    crash::protect(&panic_flush, Some(Arc::new(log)));

    let mut document = Document::default();
    for index in 0..CHANGES {
        let mut transaction = document.transaction(format!("Add p{index}"));
        transaction.add_parameter(format!("p{index}"), Expression::Number(index as f64));
        let transaction = transaction.finish();
        document.apply(transaction.clone()).unwrap();
        storage.record(JournalEntry::Apply(transaction)).unwrap();
    }

    match ending {
        Ending::Panic => panic!("crashing with unflushed changes"),
        Ending::Terminate => {
            #[cfg(unix)]
            signal_hook::low_level::raise(signal_hook::consts::SIGTERM).unwrap();
        }
        Ending::Kill => {
            assert!(storage.flusher().flush(FLUSH_TIMEOUT));
            let mut stdout = std::io::stdout().lock();
            writeln!(stdout, "{WRITTEN}").unwrap();
            stdout.flush().unwrap();
        }
    }
    loop {
        thread::park();
    }
}

#[cfg(test)]
mod child {
    use super::*;

    pub fn run(ending: Ending, dir: &Path) -> ExitStatus {
        let mut child = Command::new(env::current_exe().unwrap())
            .args([CHILD_TEST, "--exact", "--ignored", "--nocapture"])
            .env(MODE, ending.name())
            .env(DIR, dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();

        let stdout = child.stdout.take().unwrap();
        for line in BufReader::new(stdout).lines() {
            if line.unwrap() == WRITTEN {
                child.kill().unwrap();
            }
        }
        child.wait().unwrap()
    }

    pub fn assert_every_change_recovers(dir: &Path) {
        let recovered = caditor_file::scan(Some(dir), &[]);

        assert_eq!(recovered.len(), 1, "{recovered:?}");
        let recovered = &recovered[0];
        assert_eq!(recovered.changes(), CHANGES);
        assert!(recovered.issues.is_empty(), "{:?}", recovered.issues);
        assert_eq!(recovered.editor.document().parameters().len(), CHANGES);
    }

    pub fn ended_unexpectedly(dir: &Path) -> bool {
        let state = dir.join(STATE);
        let own = state.join("logs").join("none.log");
        !caditor_file::ended_unexpectedly(&state, &own).is_empty()
    }
}

#[test]
fn a_panic_flushes_the_changes_just_recorded() {
    let dir = tempfile::tempdir().unwrap();

    let status = child::run(Ending::Panic, dir.path());

    assert_eq!(status.code(), Some(PANIC_EXIT_CODE), "{status:?}");
    child::assert_every_change_recovers(dir.path());
    assert!(child::ended_unexpectedly(dir.path()));
}

#[cfg(unix)]
fn killed(status: ExitStatus) -> bool {
    use std::os::unix::process::ExitStatusExt;

    status.signal() == Some(signal_hook::consts::SIGKILL)
}

#[cfg(windows)]
fn killed(status: ExitStatus) -> bool {
    !status.success() && status.code() != Some(PANIC_EXIT_CODE)
}

#[cfg(unix)]
#[test]
fn a_termination_signal_flushes_the_changes_just_recorded_and_stops_by_that_signal() {
    use std::os::unix::process::ExitStatusExt;

    let dir = tempfile::tempdir().unwrap();

    let status = child::run(Ending::Terminate, dir.path());

    assert_eq!(
        status.signal(),
        Some(signal_hook::consts::SIGTERM),
        "{status:?}"
    );
    child::assert_every_change_recovers(dir.path());
    assert!(!child::ended_unexpectedly(dir.path()));
}

#[test]
fn a_killed_process_leaves_a_journal_the_next_start_recovers() {
    let dir = tempfile::tempdir().unwrap();

    let status = child::run(Ending::Kill, dir.path());

    assert!(killed(status), "{status:?}");
    child::assert_every_change_recovers(dir.path());
    assert!(child::ended_unexpectedly(dir.path()));
}
