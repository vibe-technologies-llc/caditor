#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

#[derive(Debug, Default)]
pub struct SseParser {
    event: Option<String>,
    data: Vec<String>,
}

impl SseParser {
    pub fn push_line(&mut self, raw: &str) -> Option<SseEvent> {
        let line = raw.trim_end_matches(['\r', '\n']);

        if line.is_empty() {
            return self.flush();
        }
        if line.starts_with(':') {
            return None;
        }

        let (field, value) = match line.split_once(':') {
            Some((field, rest)) => (field, rest.strip_prefix(' ').unwrap_or(rest)),
            None => (line, ""),
        };

        match field {
            "event" => self.event = Some(value.to_owned()),
            "data" => self.data.push(value.to_owned()),
            _ => {}
        }
        None
    }

    pub fn flush(&mut self) -> Option<SseEvent> {
        if self.data.is_empty() {
            self.event = None;
            return None;
        }

        let event = self.event.take();
        let data = self.data.join("\n");

        self.data.clear();
        Some(SseEvent { event, data })
    }
}
