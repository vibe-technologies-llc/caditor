use serde_json::{Value, json};

use crate::{
    guide::{Block, Named, Page, Span, Target},
    mcp::{
        reading,
        session::{Session, Work},
        tools::ToolError,
    },
};

const SUMMARY: &str = "caditor://model/summary";
const PARAMETERS: &str = "caditor://model/parameters";
const GUIDE: &str = "caditor://guide/";
const JSON: &str = "application/json";
const MARKDOWN: &str = "text/markdown";

#[derive(Debug, thiserror::Error)]
pub enum ResourceError {
    #[error("there is no resource {uri}; resources/list names every one")]
    NotFound { uri: String },
    #[error(transparent)]
    Reading(#[from] ToolError),
}

pub fn listing() -> Value {
    let mut resources = vec![
        json!({
            "uri": SUMMARY,
            "name": "model-summary",
            "title": "Model summary",
            "description": "The open model: file, unsaved changes, undo and redo, counts and failing features",
            "mimeType": JSON,
        }),
        json!({
            "uri": PARAMETERS,
            "name": "model-parameters",
            "title": "Parameter table",
            "description": "Every parameter with its expression, value and users",
            "mimeType": JSON,
        }),
    ];
    resources.extend(Page::ALL.into_iter().map(|page| {
        json!({
            "uri": format!("{GUIDE}{}", page.id()),
            "name": page.id(),
            "title": page.title(),
            "description": format!("The user guide's page on {}", page.title()),
            "mimeType": MARKDOWN,
        })
    }));
    json!({ "resources": resources })
}

pub fn read(session: &mut Session, work: &Work<'_>, uri: &str) -> Result<Value, ResourceError> {
    let (text, kind) = match uri {
        SUMMARY => (
            Value::Object(reading::model(session, work)?).to_string(),
            JSON,
        ),
        PARAMETERS => (
            Value::Object(reading::parameters(session)).to_string(),
            JSON,
        ),
        other => {
            let page = other
                .strip_prefix(GUIDE)
                .and_then(Page::from_id)
                .ok_or_else(|| ResourceError::NotFound {
                    uri: uri.to_owned(),
                })?;
            (page_text(page), MARKDOWN)
        }
    };
    Ok(json!({ "contents": [{ "uri": uri, "mimeType": kind, "text": text }] }))
}

pub fn page_text(page: Page) -> String {
    let mut text = format!("# {}\n", page.title());
    let mut listing = false;
    for block in page.blocks() {
        let item = matches!(block, Block::Item(_));
        if !(item && listing) {
            text.push('\n');
        }
        listing = item;
        match block {
            Block::Heading(heading) => text.push_str(&format!("## {heading}\n")),
            Block::Paragraph(spans) => text.push_str(&format!("{}\n", spans_text(spans))),
            Block::Item(spans) => text.push_str(&format!("- {}\n", spans_text(spans))),
        }
    }
    text
}

fn spans_text(spans: &[Span]) -> String {
    spans
        .iter()
        .map(|span| match span {
            Span::Text(text) => text.clone(),
            Span::Strong(text) => format!("**{text}**"),
            Span::Code(text) => format!("`{text}`"),
            Span::Link {
                text,
                target: Target::Page(page),
            } => format!("[{text}]({GUIDE}{})", page.id()),
            Span::Link {
                text,
                target: Target::Unknown(_),
            } => text.clone(),
            Span::Command(Named::Command(command)) => command.title(),
            Span::Command(Named::Unknown(id)) => id.clone(),
        })
        .collect()
}
