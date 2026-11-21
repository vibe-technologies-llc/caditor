use std::{
    collections::BTreeSet,
    panic::{self, AssertUnwindSafe},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Sender},
    },
};

use caditor_ai::{ToolDefinition, ToolResult};
use caditor_document::{Document, Evaluation};
use parking_lot::Mutex;
use serde_json::{Map, Value, json};

use crate::{
    cloud::thumbnails::Thumbnail,
    mcp::{
        bridge::{BridgeError, BridgeHandler, ToolCallOutcome},
        resources,
        server::INSTRUCTIONS,
        session::{AttachedConfig, Change, SaveFailure, Session, Stored, Work},
        tools::{Answer, Tool, ToolError},
    },
    model::Waker,
};

pub const LIST_RESOURCES_TOOL: &str = "list_resources";
pub const READ_RESOURCE_TOOL: &str = "read_resource";
const MAX_STALE_RETRIES: usize = 2;

pub struct ModelSnapshot {
    pub session_id: u64,
    pub revision: u64,
    pub document: Document,
    pub evaluation: Evaluation,
    pub path: Option<PathBuf>,
    pub undo_label: Option<String>,
    pub redo_label: Option<String>,
    pub is_unsaved: bool,
    pub load_issues: Vec<String>,
}

pub enum CommitOutcome {
    Applied,
    Stale,
    ModelChanged,
}

pub enum BridgeRequest {
    Snapshot {
        reply: Sender<ModelSnapshot>,
    },
    Commit {
        expected_session: u64,
        expected_revision: u64,
        changes: Vec<Change>,
        reply: Sender<CommitOutcome>,
    },
    Save {
        expected_session: u64,
        path: Option<PathBuf>,
        reply: Sender<Result<Stored, SaveFailure>>,
    },
}

pub enum ExecutorEvent {
    Started {
        id: u64,
        tool_name: String,
        title: String,
        arguments_preview: String,
    },
    Progress {
        id: u64,
        progress: String,
    },
    Finished {
        id: u64,
        succeeded: bool,
        summary: String,
        preview: Option<Arc<Thumbnail>>,
    },
}

pub fn system_prompt() -> String {
    format!(
        "{INSTRUCTIONS}\n\n\
         You are running directly inside the caditor editor window on the user's open model. \
         Every tool call operates live on the open model, updates the 3D viewport immediately, \
         and lands in the user's Undo history. Use list_resources and read_resource to consult \
         the caditor user guide (caditor://guide/<page>) whenever you need details on a feature \
         or constraint. After creating or editing geometry, check failing features and consider \
         calling render_view to visually verify your work. Keep your final explanations concise \
         and practical."
    )
}

pub fn all_tool_definitions() -> Vec<ToolDefinition> {
    let mut defs: Vec<ToolDefinition> = Tool::ALL
        .into_iter()
        .map(|tool| ToolDefinition {
            name: tool.name().to_owned(),
            description: tool.description().to_owned(),
            input_schema: tool.input_schema(),
        })
        .collect();

    defs.push(ToolDefinition {
        name: LIST_RESOURCES_TOOL.to_owned(),
        description: "Lists available MCP resources, including caditor://model/summary, \
                      caditor://model/parameters and every caditor://guide/<page> user guide page."
            .to_owned(),
        input_schema: json!({
            "type": "object",
            "properties": {},
            "required": [],
            "additionalProperties": false
        }),
    });
    defs.push(ToolDefinition {
        name: READ_RESOURCE_TOOL.to_owned(),
        description: "Reads an MCP resource by URI, such as caditor://guide/sketches or \
                      caditor://model/summary."
            .to_owned(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "uri": {
                    "type": "string",
                    "description": "The resource URI returned by list_resources"
                }
            },
            "required": ["uri"],
            "additionalProperties": false
        }),
    });
    defs
}

pub fn tool_title(name: &str) -> String {
    if name == LIST_RESOURCES_TOOL {
        return "List resources".to_owned();
    }
    if name == READ_RESOURCE_TOOL {
        return "Read resource".to_owned();
    }
    Tool::named(name)
        .map(|tool| tool.title().to_owned())
        .unwrap_or_else(|| name.to_owned())
}

pub struct LiveMcpExecutor {
    bridge: Sender<BridgeRequest>,
    events: Sender<ExecutorEvent>,
    wake: Arc<dyn Fn() + Send + Sync>,
    folders: Vec<PathBuf>,
    exported: Mutex<BTreeSet<PathBuf>>,
    next_tool_id: Mutex<u64>,
}

impl LiveMcpExecutor {
    pub fn new(
        bridge: Sender<BridgeRequest>,
        events: Sender<ExecutorEvent>,
        wake: Waker,
        folders: Vec<PathBuf>,
    ) -> Self {
        let wake_mutex = Mutex::new(wake);
        Self {
            bridge,
            events,
            wake: Arc::new(move || {
                (wake_mutex.lock())();
            }),
            folders,
            exported: Mutex::new(BTreeSet::new()),
            next_tool_id: Mutex::new(1),
        }
    }

    fn alloc_id(&self) -> u64 {
        let mut next = self.next_tool_id.lock();

        let id = *next;

        *next = next.saturating_add(1);
        id
    }

    fn emit(&self, event: ExecutorEvent) {
        let _ = self.events.send(event);

        (self.wake)();
    }

    fn request_snapshot(&self) -> Result<ModelSnapshot, BridgeError> {
        let (reply_tx, reply_rx) = mpsc::channel();

        self.bridge
            .send(BridgeRequest::Snapshot { reply: reply_tx })
            .map_err(|_| BridgeError::ModelUnavailable)?;
        (self.wake)();
        reply_rx.recv().map_err(|_| BridgeError::ModelUnavailable)
    }

    fn commit_changes(
        &self,
        expected_session: u64,
        expected_revision: u64,
        changes: Vec<Change>,
    ) -> Result<CommitOutcome, BridgeError> {
        let (reply_tx, reply_rx) = mpsc::channel();

        self.bridge
            .send(BridgeRequest::Commit {
                expected_session,
                expected_revision,
                changes,
                reply: reply_tx,
            })
            .map_err(|_| BridgeError::ModelUnavailable)?;
        (self.wake)();
        reply_rx.recv().map_err(|_| BridgeError::ModelUnavailable)
    }

    fn make_save_hook(&self, expected_session: u64) -> crate::mcp::session::SaveHook {
        let bridge = self.bridge.clone();
        let wake = Arc::clone(&self.wake);
        Box::new(move |path| {
            let (reply_tx, reply_rx) = mpsc::channel();

            bridge
                .send(BridgeRequest::Save {
                    expected_session,
                    path,
                    reply: reply_tx,
                })
                .map_err(|_| SaveFailure::WriterStopped)?;
            wake();
            reply_rx.recv().map_err(|_| SaveFailure::WriterStopped)?
        })
    }

    pub fn execute_for_api(
        &self,
        call_id: &str,
        name: &str,
        arguments: Value,
        cancelled: &Arc<AtomicBool>,
    ) -> ToolResult {
        if name == LIST_RESOURCES_TOOL {
            let activity_id = self.alloc_id();

            self.emit(ExecutorEvent::Started {
                id: activity_id,
                tool_name: name.to_owned(),
                title: tool_title(name),
                arguments_preview: String::new(),
            });

            let text = resources::listing().to_string();

            self.emit(ExecutorEvent::Finished {
                id: activity_id,
                succeeded: true,
                summary: "Listed MCP resources".to_owned(),
                preview: None,
            });
            return ToolResult {
                call_id: call_id.to_owned(),
                name: name.to_owned(),
                content: text,
                image_png: None,
                is_error: false,
            };
        }
        if name == READ_RESOURCE_TOOL {
            let uri = arguments
                .get("uri")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let activity_id = self.alloc_id();

            self.emit(ExecutorEvent::Started {
                id: activity_id,
                tool_name: name.to_owned(),
                title: tool_title(name),
                arguments_preview: uri.clone(),
            });
            return match self.run_resource(&uri, cancelled) {
                Ok(value) => {
                    self.emit(ExecutorEvent::Finished {
                        id: activity_id,
                        succeeded: true,
                        summary: format!("Read {uri}"),
                        preview: None,
                    });
                    ToolResult {
                        call_id: call_id.to_owned(),
                        name: name.to_owned(),
                        content: value.to_string(),
                        image_png: None,
                        is_error: false,
                    }
                }
                Err(error) => {
                    let text = error.to_string();

                    self.emit(ExecutorEvent::Finished {
                        id: activity_id,
                        succeeded: false,
                        summary: text.clone(),
                        preview: None,
                    });
                    ToolResult {
                        call_id: call_id.to_owned(),
                        name: name.to_owned(),
                        content: text,
                        image_png: None,
                        is_error: true,
                    }
                }
            };
        }
        match self.run_mcp_tool(name, arguments, cancelled) {
            Ok(answer) => {
                let text = Value::Object(answer.data).to_string();
                ToolResult {
                    call_id: call_id.to_owned(),
                    name: name.to_owned(),
                    content: text,
                    image_png: answer.image,
                    is_error: false,
                }
            }
            Err(error) => ToolResult {
                call_id: call_id.to_owned(),
                name: name.to_owned(),
                content: error.to_string(),
                image_png: None,
                is_error: true,
            },
        }
    }

    fn run_mcp_tool(
        &self,
        name: &str,
        arguments: Value,
        cancelled: &Arc<AtomicBool>,
    ) -> Result<Answer, BridgeError> {
        let activity_id = self.alloc_id();

        self.emit(ExecutorEvent::Started {
            id: activity_id,
            tool_name: name.to_owned(),
            title: tool_title(name),
            arguments_preview: preview_arguments(&arguments),
        });

        let result = self.run_mcp_tool_inner(activity_id, name, arguments, cancelled);

        match &result {
            Ok(answer) => {
                let preview = answer
                    .image
                    .as_deref()
                    .and_then(Thumbnail::from_bytes)
                    .map(Arc::new);

                self.emit(ExecutorEvent::Finished {
                    id: activity_id,
                    succeeded: true,
                    summary: summarize_answer(name, &answer.data),
                    preview,
                });
            }
            Err(error) => {
                self.emit(ExecutorEvent::Finished {
                    id: activity_id,
                    succeeded: false,
                    summary: error.to_string(),
                    preview: None,
                });
            }
        }
        result
    }

    fn run_mcp_tool_inner(
        &self,
        activity_id: u64,
        name: &str,
        arguments: Value,
        cancelled: &Arc<AtomicBool>,
    ) -> Result<Answer, BridgeError> {
        if cancelled.load(Ordering::Acquire) {
            return Err(BridgeError::Cancelled);
        }

        let tool = Tool::named(name).ok_or_else(|| {
            let err = serde_json::from_str::<Value>("invalid").err();
            match err {
                Some(source) => BridgeError::Tool(ToolError::Arguments {
                    tool: "unknown",
                    source,
                }),
                None => BridgeError::ModelUnavailable,
            }
        })?;

        for attempt in 0..=MAX_STALE_RETRIES {
            let call = tool.decode(arguments.clone())?;
            let snapshot = self.request_snapshot()?;
            let session_id = snapshot.session_id;
            let revision = snapshot.revision;
            let exported = self.exported.lock().clone();

            let mut session = Session::attached(AttachedConfig {
                document: snapshot.document,
                evaluation: snapshot.evaluation,
                model: snapshot.path,
                folders: self.folders.clone(),
                exported,
                undo_label: snapshot.undo_label,
                redo_label: snapshot.redo_label,
                is_unsaved: snapshot.is_unsaved,
                load_issues: snapshot.load_issues,
                on_save: Some(self.make_save_hook(session_id)),
            });
            let report = |message: &str| {
                self.emit(ExecutorEvent::Progress {
                    id: activity_id,
                    progress: message.to_owned(),
                });
            };
            let work = Work::new(Arc::clone(cancelled), &report);

            let outcome = panic::catch_unwind(AssertUnwindSafe(|| call.run(&mut session, &work)));
            let answer = match outcome {
                Ok(Ok(answer)) => answer,
                Ok(Err(error)) => return Err(BridgeError::Tool(error)),
                Err(_) => return Err(BridgeError::ModelUnavailable),
            };
            let changes = session.take_changes();
            let exported_after = session.exported_paths().clone();

            if changes.is_empty() {
                *self.exported.lock() = exported_after;
                return Ok(answer);
            }
            match self.commit_changes(session_id, revision, changes)? {
                CommitOutcome::Applied => {
                    *self.exported.lock() = exported_after;
                    return Ok(answer);
                }
                CommitOutcome::ModelChanged => return Err(BridgeError::ModelChanged),
                CommitOutcome::Stale if attempt < MAX_STALE_RETRIES => continue,
                CommitOutcome::Stale => return Err(BridgeError::ModelChanged),
            }
        }
        Err(BridgeError::ModelChanged)
    }

    fn run_resource(&self, uri: &str, cancelled: &Arc<AtomicBool>) -> Result<Value, BridgeError> {
        if cancelled.load(Ordering::Acquire) {
            return Err(BridgeError::Cancelled);
        }

        let snapshot = self.request_snapshot()?;
        let mut session = Session::attached(AttachedConfig {
            document: snapshot.document,
            evaluation: snapshot.evaluation,
            model: snapshot.path,
            folders: self.folders.clone(),
            exported: BTreeSet::new(),
            undo_label: snapshot.undo_label,
            redo_label: snapshot.redo_label,
            is_unsaved: snapshot.is_unsaved,
            load_issues: snapshot.load_issues,
            on_save: None,
        });
        let report = |_: &str| {};
        let work = Work::new(Arc::clone(cancelled), &report);
        resources::read(&mut session, &work, uri).map_err(BridgeError::Resource)
    }
}

impl BridgeHandler for LiveMcpExecutor {
    fn call_tool(&self, name: &str, arguments: Value, cancelled: &AtomicBool) -> ToolCallOutcome {
        let flag = Arc::new(AtomicBool::new(cancelled.load(Ordering::Acquire)));
        ToolCallOutcome {
            answer: self.run_mcp_tool(name, arguments, &flag),
        }
    }

    fn read_resource(&self, uri: &str, cancelled: &AtomicBool) -> Result<Value, BridgeError> {
        let flag = Arc::new(AtomicBool::new(cancelled.load(Ordering::Acquire)));
        self.run_resource(uri, &flag)
    }
}

fn preview_arguments(arguments: &Value) -> String {
    let text = match arguments {
        Value::Object(map) if map.is_empty() => return String::new(),
        other => other.to_string(),
    };
    truncate_chars(&text, 140)
}

fn summarize_answer(name: &str, data: &Map<String, Value>) -> String {
    if let Some(change) = data.get("change").and_then(Value::as_str) {
        let failing = data
            .get("failing")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);

        if failing > 0 {
            return format!("{change} ({failing} failing)");
        }
        return change.to_owned();
    }
    if let Some(saved) = data.get("saved").and_then(Value::as_str) {
        return format!("Saved to {saved}");
    }
    if let Some(exported) = data.get("exported").and_then(Value::as_str) {
        return format!("Exported {exported}");
    }
    if let Some(bodies) = data.get("bodies").and_then(Value::as_array) {
        return format!("{} bodies", bodies.len());
    }
    if let Some(features) = data.get("features").and_then(Value::as_array) {
        return format!("{} features", features.len());
    }
    if let Some(faces) = data.get("faces").and_then(Value::as_array) {
        return format!("{} faces", faces.len());
    }
    if let Some(edges) = data.get("edges").and_then(Value::as_array) {
        return format!("{} edges", edges.len());
    }
    if name == "render_view" {
        return "Rendered model view".to_owned();
    }
    "Done".to_owned()
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();

    let kept: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{kept}…")
    } else {
        kept
    }
}
