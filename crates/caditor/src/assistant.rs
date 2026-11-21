pub mod cli_runner;
pub mod executor;
pub mod keys;
pub mod state;

use std::{
    collections::{BTreeMap, VecDeque},
    panic::{self, AssertUnwindSafe},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
};

use caditor_ai::{
    ApiKey, ApiProvider, Message, Network, RequestConfig, StreamEvent, Transport, stream_turn,
};

pub use self::state::{
    AssistantCommand, AssistantFailure, AssistantStatus, Backend, ChatEntry, CliState, CliTool,
    EntryKind, KeySource, ToolActivity, ToolStatus,
};
use self::{
    cli_runner::{CliBackend, CliTurnRequest, SystemCliBackend, start_bridge_for_cli},
    executor::{
        BridgeRequest, CommitOutcome, ExecutorEvent, LiveMcpExecutor, ModelSnapshot,
        all_tool_definitions, system_prompt,
    },
    keys::KeyStore,
};
use crate::{
    cloud::keyring::{AiKeyring, system_ai},
    mcp::session::{Change, SaveFailure, Stored},
    model::{Action, Model, Notice, RecomputeStatus, WakerFactory},
    preferences::{AssistantPreferences, PreferenceChange, PreferencesCommand},
};

const MAX_TOOL_ROUNDS: usize = 25;
const DEFAULT_MAX_TOKENS: u32 = 8192;

pub struct AssistantServices {
    pub transport: Arc<dyn Transport>,
    pub cli: Arc<dyn CliBackend>,
    pub keyring: Arc<dyn AiKeyring>,
    pub state_dir: Option<PathBuf>,
    pub make_waker: WakerFactory,
}

impl AssistantServices {
    pub fn live(state_dir: Option<PathBuf>, make_waker: WakerFactory) -> Self {
        Self {
            transport: Arc::new(Network::new()),
            cli: Arc::new(SystemCliBackend),
            keyring: system_ai(),
            state_dir,
            make_waker,
        }
    }

    #[cfg(test)]
    pub fn for_tests(
        transport: Arc<dyn Transport>,
        cli: Arc<dyn CliBackend>,
        keyring: Arc<dyn AiKeyring>,
        state_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            transport,
            cli,
            keyring,
            state_dir,
            make_waker: Box::new(|| Box::new(|| {})),
        }
    }

    pub fn without_files() -> Self {
        #[cfg(test)]
        let keyring: Arc<dyn AiKeyring> = crate::cloud::keyring::FakeAiKeyring::unreachable();
        #[cfg(not(test))]
        let keyring: Arc<dyn AiKeyring> = system_ai();
        Self {
            transport: Arc::new(Network::new()),
            cli: Arc::new(SystemCliBackend),
            keyring,
            state_dir: None,
            make_waker: Box::new(|| Box::new(|| {})),
        }
    }
}

enum WorkerEvent {
    KeysLoaded(BTreeMap<ApiProvider, (ApiKey, KeySource)>),
    KeyUpdated {
        provider: ApiProvider,
        key: ApiKey,
        source: KeySource,
        notice: Option<Notice>,
    },
    ClisDiscovered(BTreeMap<CliTool, CliState>),
    TextDelta {
        turn_id: u64,
        chunk: String,
    },
    Executor {
        turn_id: u64,
        event: ExecutorEvent,
    },
    TurnFinished {
        turn_id: u64,
        history: Vec<Message>,
        cli_session: Option<String>,
        failure: Option<AssistantFailure>,
    },
}

struct PendingSave {
    reply: Sender<Result<Stored, SaveFailure>>,
    target: PathBuf,
}

pub struct Assistant {
    pub open: bool,
    pub composer: String,
    pub focus_composer: bool,
    entries: Vec<ChatEntry>,
    api_history: Vec<Message>,
    cli_sessions: BTreeMap<CliTool, String>,
    keys: BTreeMap<ApiProvider, (ApiKey, KeySource)>,
    clis: BTreeMap<CliTool, CliState>,
    status: AssistantStatus,
    active_turn: Option<ActiveTurn>,
    next_entry_id: u64,
    next_turn_id: u64,
    last_prompt: Option<String>,
    observed_session: Option<u64>,
    services: AssistantServices,
    events_tx: Sender<WorkerEvent>,
    events_rx: Receiver<WorkerEvent>,
    bridge_tx: Sender<BridgeRequest>,
    bridge_rx: Receiver<BridgeRequest>,
    pending_snapshot: Option<Sender<ModelSnapshot>>,
    pending_save: Option<PendingSave>,
    keys_requested: bool,
    clis_requested: bool,
}

struct ActiveTurn {
    id: u64,
    entry_id: u64,
    cancel: Arc<AtomicBool>,
}

impl Assistant {
    pub fn new(services: AssistantServices) -> Self {
        let (events_tx, events_rx) = mpsc::channel();
        let (bridge_tx, bridge_rx) = mpsc::channel();
        let mut keys = BTreeMap::new();

        for provider in ApiProvider::ALL {
            keys.insert(provider, (ApiKey::default(), KeySource::Missing));
        }

        let mut clis = BTreeMap::new();

        for tool in CliTool::ALL {
            clis.insert(tool, CliState::Checking);
        }
        Self {
            open: false,
            composer: String::new(),
            focus_composer: false,
            entries: Vec::new(),
            api_history: Vec::new(),
            cli_sessions: BTreeMap::new(),
            keys,
            clis,
            status: AssistantStatus::Idle,
            active_turn: None,
            next_entry_id: 1,
            next_turn_id: 1,
            last_prompt: None,
            observed_session: None,
            services,
            events_tx,
            events_rx,
            bridge_tx,
            bridge_rx,
            pending_snapshot: None,
            pending_save: None,
            keys_requested: false,
            clis_requested: false,
        }
    }

    pub fn set_services(&mut self, services: AssistantServices, prefs: &AssistantPreferences) {
        self.cancel();
        self.services = services;
        self.keys_requested = false;
        self.clis_requested = false;
        self.start_loading_keys();
        self.refresh_clis(prefs);
    }

    pub fn entries(&self) -> &[ChatEntry] {
        &self.entries
    }

    pub fn status(&self) -> &AssistantStatus {
        &self.status
    }

    pub fn is_busy(&self) -> bool {
        self.status.is_busy()
    }

    pub fn key_source(&self, provider: ApiProvider) -> KeySource {
        self.keys
            .get(&provider)
            .map_or(KeySource::Missing, |(_, source)| *source)
    }

    pub fn cli_state(&self, tool: CliTool) -> &CliState {
        self.clis.get(&tool).unwrap_or(&CliState::Checking)
    }

    pub fn backend_ready(&self, backend: Backend) -> bool {
        match backend {
            Backend::Api(provider) => {
                !provider.requires_key() || self.key_source(provider).is_available()
            }
            Backend::Cli(tool) => self.cli_state(tool).is_available(),
        }
    }

    pub fn can_retry(&self) -> bool {
        !self.is_busy() && self.last_prompt.is_some()
    }

    pub fn ensure_loaded(&mut self, prefs: &AssistantPreferences) {
        if !self.keys_requested {
            self.start_loading_keys();
        }
        if !self.clis_requested {
            self.refresh_clis(prefs);
        }
    }

    pub fn toggle(&mut self, prefs: &AssistantPreferences) {
        self.open = !self.open;
        if self.open {
            self.focus_composer = true;
            self.ensure_loaded(prefs);
        }
    }

    pub fn cancel(&mut self) {
        if let Some(active) = self.active_turn.take() {
            active.cancel.store(true, Ordering::Release);
            if let Some(entry) = self
                .entries
                .iter_mut()
                .find(|entry| entry.id == active.entry_id)
                && entry.text.is_empty()
                && entry.tools.is_empty()
                && entry.failure.is_none()
            {
                entry.text = "Stopped.".to_owned();
            }
        }
        self.status = AssistantStatus::Idle;
        self.pending_snapshot = None;
        if let Some(save) = self.pending_save.take() {
            let _ = save.reply.send(Err(SaveFailure::WriterStopped));
        }
    }

    pub fn after_failed_frame(&mut self) {
        self.cancel();
        self.open = false;
    }

    pub fn shutdown(&mut self) {
        self.cancel();
    }

    pub fn note_model_session(&mut self, model: &Model) {
        let session = model.session();
        if self
            .observed_session
            .replace(session)
            .is_some_and(|old| old != session)
            && !self.entries.is_empty()
        {
            let id = self.alloc_entry_id();

            self.entries.push(ChatEntry {
                id,
                kind: EntryKind::Notice,
                text: format!("Switched to {}.", model.display_label()),
                tools: Vec::new(),
                failure: None,
            });
        }
    }

    pub fn perform(
        &mut self,
        command: AssistantCommand,
        prefs: &AssistantPreferences,
        actions: &mut Vec<Action>,
    ) {
        match command {
            AssistantCommand::OpenSettings => {
                self.ensure_loaded(prefs);
                actions.push(Action::Preferences(PreferencesCommand::Show));
            }
            AssistantCommand::SelectBackend(backend) => {
                actions.push(Action::Preferences(PreferencesCommand::Change(
                    PreferenceChange::AssistantBackend(backend),
                )));
            }
            AssistantCommand::SetModel(provider, model) => {
                actions.push(Action::Preferences(PreferencesCommand::Change(
                    PreferenceChange::AssistantModel(provider, model),
                )));
            }
            AssistantCommand::SendPrompt(prompt) => {
                self.start_turn(prompt, prefs);
            }
            AssistantCommand::Cancel => {
                self.cancel();
            }
            AssistantCommand::ClearChat => {
                self.cancel();
                self.entries.clear();
                self.api_history.clear();
                self.cli_sessions.clear();
                self.last_prompt = None;
            }
            AssistantCommand::RetryLast => {
                if let Some(prompt) = self.last_prompt.clone() {
                    self.start_turn(prompt, prefs);
                }
            }
            AssistantCommand::SaveApiKey(provider, raw) => {
                self.spawn_save_key(provider, ApiKey::new(raw));
            }
            AssistantCommand::ClearApiKey(provider) => {
                self.spawn_clear_key(provider);
            }
            AssistantCommand::RefreshClis => {
                self.refresh_clis(prefs);
            }
        }
    }

    pub fn poll_ui(&mut self, model: &Model, actions: &mut Vec<Action>) {
        self.note_model_session(model);
        self.drain_events(actions);
    }

    pub fn poll_model(&mut self, model: &mut Model, actions: &mut VecDeque<Action>) {
        self.note_model_session(model);

        let mut drained = Vec::new();

        self.drain_events(&mut drained);
        actions.extend(drained);
        for outcome in model.take_save_outcomes() {
            self.observe_save_outcome(outcome);
        }
        self.drain_bridge(model, actions);
        self.fulfill_pending_snapshot(model);
    }

    fn observe_save_outcome(&mut self, outcome: crate::model::SaveOutcome) {
        let Some(pending) = &self.pending_save else {
            return;
        };
        match outcome {
            crate::model::SaveOutcome::Saved {
                path,
                backup,
                dropped_for_size,
            } if path == pending.target => {
                if let Some(save) = self.pending_save.take() {
                    let _ = save.reply.send(Ok(Stored {
                        backup,
                        dropped_for_size,
                    }));
                }
            }
            crate::model::SaveOutcome::ChangedOnDisk { path } if path == pending.target => {
                if let Some(save) = self.pending_save.take() {
                    let _ = save.reply.send(Err(SaveFailure::ChangedOnDisk {
                        file: crate::model::display_name(Some(&path)),
                    }));
                }
            }
            crate::model::SaveOutcome::Failed { path, error } if path == pending.target => {
                if let Some(save) = self.pending_save.take() {
                    let _ = save.reply.send(Err(SaveFailure::Writing {
                        file: crate::model::display_name(Some(&path)),
                        source: error,
                    }));
                }
            }
            _ => {}
        }
    }

    fn alloc_entry_id(&mut self) -> u64 {
        let id = self.next_entry_id;

        self.next_entry_id = self.next_entry_id.saturating_add(1);
        id
    }

    fn start_loading_keys(&mut self) {
        self.keys_requested = true;

        let store = KeyStore::new(
            self.services.state_dir.as_deref(),
            Arc::clone(&self.services.keyring),
        );
        let tx = self.events_tx.clone();
        let wake = (self.services.make_waker)();

        let _ = thread::Builder::new()
            .name("assistant keys".to_owned())
            .spawn(move || {
                let loaded = store.load_all();
                let _ = tx.send(WorkerEvent::KeysLoaded(loaded));

                wake();
            });
    }

    fn spawn_save_key(&mut self, provider: ApiProvider, key: ApiKey) {
        let store = KeyStore::new(
            self.services.state_dir.as_deref(),
            Arc::clone(&self.services.keyring),
        );
        let tx = self.events_tx.clone();
        let wake = (self.services.make_waker)();

        let _ = thread::Builder::new()
            .name("assistant save key".to_owned())
            .spawn(move || {
                let (saved_key, source, notice) = match store.save(provider, &key) {
                    Ok(source) => (
                        key,
                        source,
                        Some(Notice::success(format!("Saved the {provider} API key."))),
                    ),
                    Err(error) => {
                        let (existing, existing_source) = store.load(provider);
                        (
                            existing,
                            existing_source,
                            Some(Notice::failure(format!(
                                "Could not save the {provider} API key: {error}"
                            ))),
                        )
                    }
                };

                let _ = tx.send(WorkerEvent::KeyUpdated {
                    provider,
                    key: saved_key,
                    source,
                    notice,
                });

                wake();
            });
    }

    fn spawn_clear_key(&mut self, provider: ApiProvider) {
        let store = KeyStore::new(
            self.services.state_dir.as_deref(),
            Arc::clone(&self.services.keyring),
        );
        let tx = self.events_tx.clone();
        let wake = (self.services.make_waker)();

        let _ = thread::Builder::new()
            .name("assistant clear key".to_owned())
            .spawn(move || {
                let notice = match store.clear(provider) {
                    Ok(()) => Some(Notice::info(format!(
                        "Removed the saved {provider} API key."
                    ))),
                    Err(error) => Some(Notice::warning(format!(
                        "Could not remove the {provider} API key: {error}"
                    ))),
                };
                let (key, source) = store.load(provider);

                let _ = tx.send(WorkerEvent::KeyUpdated {
                    provider,
                    key,
                    source,
                    notice,
                });

                wake();
            });
    }

    pub fn refresh_clis(&mut self, prefs: &AssistantPreferences) {
        self.clis_requested = true;
        for tool in CliTool::ALL {
            self.clis.insert(tool, CliState::Checking);
        }

        let cli = Arc::clone(&self.services.cli);
        let overrides = [
            (CliTool::ClaudeCode, prefs.claude_cli.clone()),
            (CliTool::GeminiCli, prefs.gemini_cli.clone()),
            (CliTool::CodexCli, prefs.codex_cli.clone()),
        ];
        let tx = self.events_tx.clone();
        let wake = (self.services.make_waker)();

        let _ = thread::Builder::new()
            .name("assistant cli discovery".to_owned())
            .spawn(move || {
                let mut discovered = BTreeMap::new();

                for (tool, custom) in overrides {
                    discovered.insert(tool, cli.discover(tool, &custom));
                }

                let _ = tx.send(WorkerEvent::ClisDiscovered(discovered));

                wake();
            });
    }

    fn start_turn(&mut self, raw_prompt: String, prefs: &AssistantPreferences) {
        let prompt = raw_prompt.trim().to_owned();

        if prompt.is_empty() || self.is_busy() {
            return;
        }
        self.ensure_loaded(prefs);
        self.last_prompt = Some(prompt.clone());

        let user_id = self.alloc_entry_id();

        self.entries.push(ChatEntry {
            id: user_id,
            kind: EntryKind::User,
            text: prompt.clone(),
            tools: Vec::new(),
            failure: None,
        });

        let assistant_id = self.alloc_entry_id();

        self.entries.push(ChatEntry {
            id: assistant_id,
            kind: EntryKind::Assistant,
            text: String::new(),
            tools: Vec::new(),
            failure: None,
        });

        let turn_id = self.next_turn_id;

        self.next_turn_id = self.next_turn_id.saturating_add(1);

        let cancel = Arc::new(AtomicBool::new(false));

        self.active_turn = Some(ActiveTurn {
            id: turn_id,
            entry_id: assistant_id,
            cancel: Arc::clone(&cancel),
        });
        self.status = AssistantStatus::Thinking;

        let tx = self.events_tx.clone();
        let bridge_tx = self.bridge_tx.clone();
        let wake_worker = (self.services.make_waker)();
        let wake_exec = (self.services.make_waker)();
        let folders = prefs.folders.clone();

        match prefs.backend {
            Backend::Api(provider) => {
                let api_key = self
                    .keys
                    .get(&provider)
                    .map(|(key, _)| key.clone())
                    .unwrap_or_default();
                let model = prefs.model_for(provider).to_owned();
                let base_url =
                    (provider == ApiProvider::Custom).then(|| prefs.custom_base_url.clone());
                let mut history = self.api_history.clone();

                history.push(Message::User { text: prompt });

                let transport = Arc::clone(&self.services.transport);
                let spawned = thread::Builder::new()
                    .name("assistant turn".to_owned())
                    .spawn(move || {
                        run_api_turn(ApiTurnParams {
                            turn_id,
                            provider,
                            model,
                            api_key,
                            base_url,
                            history,
                            folders,
                            transport,
                            cancel,
                            tx: tx.clone(),
                            bridge_tx,
                            wake_exec,
                            wake_worker: &*wake_worker,
                        });
                    });
                if spawned.is_err() {
                    self.fail_turn_immediately(assistant_id, AssistantFailure::ThreadUnavailable);
                }
            }
            Backend::Cli(tool) => {
                let cli = Arc::clone(&self.services.cli);
                let override_path = prefs.cli_program(tool).to_owned();
                let discovered = self.clis.get(&tool).cloned();
                let session_id = self.cli_sessions.get(&tool).cloned();
                let state_dir = self.services.state_dir.clone();

                let spawned = thread::Builder::new()
                    .name("assistant cli turn".to_owned())
                    .spawn(move || {
                        run_cli_turn(CliWorkerParams {
                            turn_id,
                            tool,
                            override_path,
                            discovered,
                            prompt,
                            session_id,
                            folders,
                            state_dir,
                            cli,
                            cancel,
                            tx: tx.clone(),
                            bridge_tx,
                            wake_exec,
                            wake_worker: &*wake_worker,
                        });
                    });
                if spawned.is_err() {
                    self.fail_turn_immediately(assistant_id, AssistantFailure::ThreadUnavailable);
                }
            }
        }
    }

    fn fail_turn_immediately(&mut self, entry_id: u64, failure: AssistantFailure) {
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == entry_id) {
            entry.failure = Some(failure);
        }
        self.active_turn = None;
        self.status = AssistantStatus::Idle;
    }

    fn drain_events(&mut self, actions: &mut Vec<Action>) {
        while let Ok(event) = self.events_rx.try_recv() {
            match event {
                WorkerEvent::KeysLoaded(loaded) => {
                    self.keys = loaded;
                }
                WorkerEvent::KeyUpdated {
                    provider,
                    key,
                    source,
                    notice,
                } => {
                    self.keys.insert(provider, (key, source));
                    if let Some(notice) = notice {
                        actions.push(Action::Inform(notice));
                    }
                }
                WorkerEvent::ClisDiscovered(discovered) => {
                    self.clis = discovered;
                }
                WorkerEvent::TextDelta { turn_id, chunk } => {
                    let Some(active) = &self.active_turn else {
                        continue;
                    };

                    if active.id != turn_id {
                        continue;
                    }
                    self.status = AssistantStatus::Thinking;
                    if let Some(entry) = self
                        .entries
                        .iter_mut()
                        .find(|entry| entry.id == active.entry_id)
                    {
                        entry.text.push_str(&chunk);
                    }
                }
                WorkerEvent::Executor { turn_id, event } => {
                    let Some(active) = &self.active_turn else {
                        continue;
                    };

                    if active.id != turn_id {
                        continue;
                    }

                    let entry = self
                        .entries
                        .iter_mut()
                        .find(|entry| entry.id == active.entry_id);
                    match event {
                        ExecutorEvent::Started {
                            id,
                            tool_name,
                            title,
                            arguments_preview,
                        } => {
                            self.status = AssistantStatus::RunningTool {
                                title: title.clone(),
                                progress: None,
                            };
                            if let Some(entry) = entry {
                                entry.tools.push(ToolActivity {
                                    id,
                                    tool_name,
                                    title,
                                    arguments_preview,
                                    status: ToolStatus::Running,
                                    progress: None,
                                    summary: String::new(),
                                    preview: None,
                                });
                            }
                        }
                        ExecutorEvent::Progress { id, progress } => {
                            if let AssistantStatus::RunningTool {
                                progress: active_progress,
                                ..
                            } = &mut self.status
                            {
                                *active_progress = Some(progress.clone());
                            }
                            if let Some(entry) = entry
                                && let Some(tool) =
                                    entry.tools.iter_mut().find(|tool| tool.id == id)
                            {
                                tool.progress = Some(progress);
                            }
                        }
                        ExecutorEvent::Finished {
                            id,
                            succeeded,
                            summary,
                            preview,
                        } => {
                            self.status = AssistantStatus::Thinking;
                            if let Some(entry) = entry
                                && let Some(tool) =
                                    entry.tools.iter_mut().find(|tool| tool.id == id)
                            {
                                tool.status = if succeeded {
                                    ToolStatus::Succeeded
                                } else {
                                    ToolStatus::Failed
                                };
                                tool.progress = None;
                                tool.summary = summary;
                                tool.preview = preview;
                            }
                        }
                    }
                }
                WorkerEvent::TurnFinished {
                    turn_id,
                    history,
                    cli_session,
                    failure,
                } => {
                    let Some(active) = &self.active_turn else {
                        continue;
                    };

                    if active.id != turn_id {
                        continue;
                    }

                    let entry_id = active.entry_id;

                    self.active_turn = None;
                    self.status = AssistantStatus::Idle;
                    if !history.is_empty() {
                        self.api_history = history;
                    }
                    if let Some(session_id) = cli_session {
                        self.cli_sessions.insert(CliTool::ClaudeCode, session_id);
                    }
                    if let Some(failure) = failure
                        && let Some(entry) =
                            self.entries.iter_mut().find(|entry| entry.id == entry_id)
                    {
                        entry.failure = Some(failure);
                    }
                }
            }
        }
    }

    fn drain_bridge(&mut self, model: &mut Model, actions: &mut VecDeque<Action>) {
        while let Ok(request) = self.bridge_rx.try_recv() {
            match request {
                BridgeRequest::Snapshot { reply } => {
                    self.pending_snapshot = Some(reply);
                }
                BridgeRequest::Commit {
                    expected_session,
                    expected_revision,
                    changes,
                    reply,
                } => {
                    if model.session() != expected_session {
                        let _ = reply.send(CommitOutcome::ModelChanged);

                        continue;
                    }
                    if model.revision() != expected_revision {
                        let _ = reply.send(CommitOutcome::Stale);

                        continue;
                    }
                    for change in changes {
                        match change {
                            Change::Apply(transaction) => {
                                actions.push_back(Action::Apply(transaction));
                            }
                            Change::Undo => actions.push_back(Action::Undo),
                            Change::Redo => actions.push_back(Action::Redo),
                        }
                    }

                    let _ = reply.send(CommitOutcome::Applied);
                }
                BridgeRequest::Save {
                    expected_session,
                    path,
                    reply,
                } => {
                    if model.session() != expected_session {
                        let _ = reply.send(Err(SaveFailure::WriterStopped));

                        continue;
                    }

                    let target = match path.or_else(|| model.path().map(PathBuf::from)) {
                        Some(target) => target,
                        None => {
                            let _ = reply.send(Err(SaveFailure::NoFilePath));

                            continue;
                        }
                    };

                    model.save_to(target.clone());
                    self.pending_save = Some(PendingSave { reply, target });
                }
            }
        }
    }

    fn fulfill_pending_snapshot(&mut self, model: &Model) {
        if matches!(model.status(), RecomputeStatus::Running { .. }) {
            return;
        }

        let Some(reply) = self.pending_snapshot.take() else {
            return;
        };
        let snapshot = ModelSnapshot {
            session_id: model.session(),
            revision: model.revision(),
            document: model.document().clone(),
            evaluation: model.evaluation().clone(),
            path: model.path().map(PathBuf::from),
            undo_label: model.undo_label().map(str::to_owned),
            redo_label: model.redo_label().map(str::to_owned),
            is_unsaved: model.is_dirty(),
            load_issues: Vec::new(),
        };

        let _ = reply.send(snapshot);
    }
}

struct ApiTurnParams<'a> {
    turn_id: u64,
    provider: ApiProvider,
    model: String,
    api_key: ApiKey,
    base_url: Option<String>,
    history: Vec<Message>,
    folders: Vec<PathBuf>,
    transport: Arc<dyn Transport>,
    cancel: Arc<AtomicBool>,
    tx: Sender<WorkerEvent>,
    bridge_tx: Sender<BridgeRequest>,
    wake_exec: crate::model::Waker,
    wake_worker: &'a (dyn Fn() + Send),
}

fn run_api_turn(params: ApiTurnParams<'_>) {
    let ApiTurnParams {
        turn_id,
        provider,
        model,
        api_key,
        base_url,
        mut history,
        folders,
        transport,
        cancel,
        tx,
        bridge_tx,
        wake_exec,
        wake_worker,
    } = params;
    let (exec_tx, exec_rx) = mpsc::channel();
    let forward_tx = tx.clone();

    let forwarder = thread::Builder::new()
        .name("assistant events".to_owned())
        .spawn(move || {
            for event in exec_rx {
                let _ = forward_tx.send(WorkerEvent::Executor { turn_id, event });
            }
        })
        .ok();

    let executor = LiveMcpExecutor::new(bridge_tx, exec_tx, wake_exec, folders);
    let tools = all_tool_definitions();
    let config = RequestConfig {
        provider,
        model,
        api_key,
        base_url,
        system_prompt: system_prompt(),
        max_tokens: DEFAULT_MAX_TOKENS,
    };

    let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
        for _ in 0..MAX_TOOL_ROUNDS {
            if cancel.load(Ordering::Acquire) {
                return Err(AssistantFailure::Api(caditor_ai::AiError::Cancelled));
            }

            let turn = stream_turn(
                &*transport,
                &config,
                &history,
                &tools,
                &cancel,
                &mut |event| {
                    if let StreamEvent::TextDelta(chunk) = event {
                        let _ = tx.send(WorkerEvent::TextDelta { turn_id, chunk });

                        wake_worker();
                    }
                },
            )?;
            let calls = turn.tool_calls.clone();

            history.push(Message::Assistant {
                text: turn.text,
                tool_calls: turn.tool_calls,
            });
            if calls.is_empty() {
                return Ok(());
            }

            let mut results = Vec::with_capacity(calls.len());

            for call in calls {
                if cancel.load(Ordering::Acquire) {
                    return Err(AssistantFailure::Api(caditor_ai::AiError::Cancelled));
                }

                let result =
                    executor.execute_for_api(&call.id, &call.name, call.arguments, &cancel);

                results.push(result);
            }
            history.push(Message::ToolResults(results));
        }
        Err(AssistantFailure::StepLimitReached)
    }));

    drop(executor);
    if let Some(handle) = forwarder {
        let _ = handle.join();
    }

    let failure = match outcome {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(error),
        Err(_) => Some(AssistantFailure::WorkerPanicked),
    };
    let _ = tx.send(WorkerEvent::TurnFinished {
        turn_id,
        history,
        cli_session: None,
        failure,
    });

    wake_worker();
}

struct CliWorkerParams<'a> {
    turn_id: u64,
    tool: CliTool,
    override_path: String,
    discovered: Option<CliState>,
    prompt: String,
    session_id: Option<String>,
    folders: Vec<PathBuf>,
    state_dir: Option<PathBuf>,
    cli: Arc<dyn CliBackend>,
    cancel: Arc<AtomicBool>,
    tx: Sender<WorkerEvent>,
    bridge_tx: Sender<BridgeRequest>,
    wake_exec: crate::model::Waker,
    wake_worker: &'a (dyn Fn() + Send),
}

fn run_cli_turn(params: CliWorkerParams<'_>) {
    let CliWorkerParams {
        turn_id,
        tool,
        override_path,
        discovered,
        prompt,
        session_id,
        folders,
        state_dir,
        cli,
        cancel,
        tx,
        bridge_tx,
        wake_exec,
        wake_worker,
    } = params;

    let binary = match discovered {
        Some(CliState::Found(path)) => path,
        _ => match cli.discover(tool, &override_path) {
            CliState::Found(path) => path,
            CliState::Missing { expected } => {
                let _ = tx.send(WorkerEvent::TurnFinished {
                    turn_id,
                    history: Vec::new(),
                    cli_session: None,
                    failure: Some(AssistantFailure::CliMissing {
                        tool,
                        binary: expected,
                    }),
                });

                wake_worker();
                return;
            }
            CliState::Checking => {
                let _ = tx.send(WorkerEvent::TurnFinished {
                    turn_id,
                    history: Vec::new(),
                    cli_session: None,
                    failure: Some(AssistantFailure::CliMissing {
                        tool,
                        binary: tool.default_binary().to_owned(),
                    }),
                });

                wake_worker();
                return;
            }
        },
    };

    let (exec_tx, exec_rx) = mpsc::channel();
    let forward_tx = tx.clone();

    let forwarder = thread::Builder::new()
        .name("assistant cli events".to_owned())
        .spawn(move || {
            for event in exec_rx {
                let _ = forward_tx.send(WorkerEvent::Executor { turn_id, event });
            }
        })
        .ok();

    let executor = Arc::new(LiveMcpExecutor::new(bridge_tx, exec_tx, wake_exec, folders));
    let mut bridge = match start_bridge_for_cli(Arc::clone(&executor)) {
        Ok(bridge) => bridge,
        Err(failure) => {
            drop(executor);
            if let Some(handle) = forwarder {
                let _ = handle.join();
            }

            let _ = tx.send(WorkerEvent::TurnFinished {
                turn_id,
                history: Vec::new(),
                cli_session: None,
                failure: Some(failure),
            });

            wake_worker();
            return;
        }
    };

    let sys_prompt = system_prompt();
    let request = CliTurnRequest {
        tool,
        binary,
        prompt: &prompt,
        system_prompt: &sys_prompt,
        session_id: session_id.as_deref(),
        bridge_port: bridge.port(),
        bridge_token: bridge.token(),
        state_dir: state_dir.as_deref(),
    };
    let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
        cli.run_turn(&request, &cancel, &mut |chunk| {
            let _ = tx.send(WorkerEvent::TextDelta { turn_id, chunk });

            wake_worker();
        })
    }));

    bridge.shutdown();
    drop(executor);
    if let Some(handle) = forwarder {
        let _ = handle.join();
    }

    let (next_session, failure) = match outcome {
        Ok(Ok(result)) => (result.session_id, None),
        Ok(Err(error)) => (None, Some(error)),
        Err(_) => (None, Some(AssistantFailure::WorkerPanicked)),
    };

    let _ = tx.send(WorkerEvent::TurnFinished {
        turn_id,
        history: Vec::new(),
        cli_session: next_session,
        failure,
    });

    wake_worker();
}
