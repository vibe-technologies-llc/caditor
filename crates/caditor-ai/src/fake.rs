use std::{
    collections::VecDeque,
    sync::atomic::{AtomicBool, Ordering},
};

use parking_lot::Mutex;

use crate::{
    AiError,
    transport::{HttpRequest, StreamStatus, Transport},
};

pub struct ScriptedStream {
    pub status: StreamStatus,
    pub lines: Vec<String>,
}

impl ScriptedStream {
    pub fn ok(lines: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            status: StreamStatus {
                status: 200,
                retry_after_seconds: None,
                error_body: None,
            },
            lines: lines.into_iter().map(Into::into).collect(),
        }
    }

    pub fn error(status: u16, body: impl Into<String>) -> Self {
        Self {
            status: StreamStatus {
                status,
                retry_after_seconds: None,
                error_body: Some(body.into()),
            },
            lines: Vec::new(),
        }
    }
}

#[derive(Default)]
pub struct ScriptedTransport {
    replies: Mutex<VecDeque<Result<ScriptedStream, AiError>>>,
    recorded: Mutex<Vec<HttpRequest>>,
}

impl ScriptedTransport {
    pub fn new(replies: impl IntoIterator<Item = Result<ScriptedStream, AiError>>) -> Self {
        Self {
            replies: Mutex::new(replies.into_iter().collect()),
            recorded: Mutex::new(Vec::new()),
        }
    }

    pub fn push(&self, reply: Result<ScriptedStream, AiError>) {
        self.replies.lock().push_back(reply);
    }

    pub fn recorded(&self) -> Vec<HttpRequest> {
        self.recorded.lock().clone()
    }
}

impl Transport for ScriptedTransport {
    fn post_stream(
        &self,
        request: &HttpRequest,
        cancel: &AtomicBool,
        on_line: &mut dyn FnMut(&str) -> Result<(), AiError>,
    ) -> Result<StreamStatus, AiError> {
        if cancel.load(Ordering::Acquire) {
            return Err(AiError::Cancelled);
        }
        self.recorded.lock().push(request.clone());
        let reply = self
            .replies
            .lock()
            .pop_front()
            .ok_or(AiError::Unreachable)??;
        if !(200..300).contains(&reply.status.status) {
            return Ok(reply.status);
        }
        for line in &reply.lines {
            if cancel.load(Ordering::Acquire) {
                return Err(AiError::Cancelled);
            }
            on_line(line)?;
        }
        Ok(reply.status)
    }
}
