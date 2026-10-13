---
paths:
  - "crates/caditor/src/mcp.rs"
  - "crates/caditor/src/mcp/**"
  - "crates/caditor/guide/mcp.md"
---

# MCP server

`caditor mcp MODEL [--folder DIR]...` (`cli::Serving`, `mcp::run`) serves one model to an AI
assistant over the Model Context Protocol on stdin and stdout, with no window, logging to stderr
and the session log as the app does. The user guide page is `guide/mcp.md`.

## Protocol (`json_rpc.rs`, `server.rs`)

- The protocol layer is our own, not the `rmcp` SDK: MCP over stdio is newline-delimited JSON-RPC
  that `serde_json` and one reader thread serve, while `rmcp` would bring tokio and macros into a
  workspace with no async runtime. The price is following each protocol revision by hand
  (`PROTOCOL_VERSIONS`). It lives in the app crate, not a crate of its own, since every tool reuses
  the app's headless, field, sketch placement, extrusion detection and hole code.
- Hand-written JSON-RPC 2.0 on `serde_json`, one message per line. `json_rpc::read_line` reads at
  most `MAX_LINE_BYTES` a line (a longer one is dropped to its newline and refused as a parse
  error); `json_rpc::parse` turns a line into `Incoming` or a typed `Rejection` (parse error,
  invalid request; batches are refused), never panicking. The `mcp` fuzz target feeds it bytes and
  decodes tool arguments from them (`caditor::fuzzing::mcp_lines`).
- A reader thread (`read_messages`) parses lines and hands them to the serving thread over a
  channel. It answers `notifications/cancelled` itself by setting the request's flag in
  `Cancellations` (registered when the request is read, so a queued request is cancelled too); a
  cancelled request gets no response, as the protocol asks. Everything else runs in order on the
  serving thread, the only writer of stdout.
- `initialize` agrees on the client's version when it is one of `PROTOCOL_VERSIONS`, else offers
  the newest; `structuredContent` is sent only from `2025-06-18` on. Offered: `ping`,
  `tools/list`, `tools/call`, `resources/list`, `resources/templates/list` (empty),
  `resources/read`. An unknown tool is `-32602`, an unknown resource `-32002`.
- A call with `_meta.progressToken` gets `notifications/progress` with a step count that only
  grows and a message ("Recomputing: 3 of 7 features", "Measuring Plate (1 of 2)").
- A panic in a call is caught (`catch_unwind`): the answer is an internal error, the evaluation is
  forgotten so the next call recomputes, and the model keeps every change made before the call
  (`Editor::apply` is atomic).

## Session (`session.rs`)

- `Session` is the one layer every front shares: an `Editor` over the document, a `Recompute`
  cache with the last `Evaluation`, and a `Storage` worker journaling every change as the window's
  `Model` does (`JournalEntry::Apply`, `Undo`, `Redo`; restarted when it stops; rebases and saves
  followed from its reports). Changes go only through `Session::perform(Change)`, the headless
  counterpart of `Action::Apply`, `Undo` and `Redo`, so a tool cannot break the model's rules.
  The automation entry in `docs/FEATURES.md` is meant to reuse it.
- Opening refuses anything but a `.caditor` path in an existing folder, a model whose journal
  another caditor holds (`FileJournal::InUse`) and one with recoverable changes
  (`FileJournal::Recoverable`), so nothing is overwritten that the window would offer to restore.
  A path that does not exist starts an empty, unsaved model.
- `evaluate` recomputes only when the revision moved, with display meshes (rendering and extrusion
  detection need them), under the call's cancel token; a cancelled recompute leaves the session
  unevaluated, so the next call starts again from the cache. Every call that answers about the
  model, writes included, waits for the recompute it caused.
- `save` sends a `SaveRequest` to the storage worker (atomic, keeping the replaced version, the
  Version history preference's retention, refusing a file changed on disk since it was opened)
  and waits for its report. Closing discards the journal only when the model is saved; otherwise
  it stays for the window to recover.

## Tools (`tools.rs`, `reading.rs`, `writing.rs`, `files.rs`)

- `Tool` lists every tool (`all_variants!`); `Tool::decode` turns arguments into a typed `Call`
  through `serde` (`deny_unknown_fields`), and `Call::run` does it. Errors are `ToolError` and the
  per-area enums (`ReferenceError`, `SketchingError`, `ModellingError`, `FileError`,
  `RenderError`, `ChangeError`), sent as a tool result with `isError` and the message as a
  sentence; the app's own refusals (field parsing, attachment, constraint conflicts) keep their
  words.
- References are by stable identity only (`references.rs`): features and bodies by id or name,
  sketch entities by id or `origin`, `horizontal_axis`, `vertical_axis`, faces by a key of their
  `FaceName` digest and occurrence (`face_key_text`), resolved on the body as it stands now.
- Every write is one transaction, one undo step, built with the app's own pieces: sketches as New
  sketch makes them (`sketch_placement::attachment_in` on a face), geometry and constraints after
  settling the sketch to its last result as sketch tools do, constraints tried first with
  `constraint_trial::conflict`, extrusions through `extrusion_detection::extrusion` unless an
  operation is given (a cut from a sketch on the cut body's face runs into it), holes from
  `hole_tools::new_hole`, the swept sketch hidden as the tools hide it. Values are parsed by
  `field::parse_expression` in millimetres and degrees. A write answers with its change's label
  and every failing feature with the tree's reason and remedy, marked `new` when the change made it
  fail.
- Reads: the feature tree with `state_of`'s words, parameters, bodies with `exact_mass` and
  `extent` under the kernel interrupt (`interruptible`), faces (at most `MOST_FACES`), a sketch
  with `SketchSummary`, and a picture through `snapshot::looking_from` and `png_bytes`, sized
  `SMALLEST_SIDE` to `LARGEST_SIDE`.
- Files: `save` writes the model's own file; `export` writes `headless::deliver`'s formats only
  into the model's folder, the `--folder` folders and folders below them (`files::allowed`:
  canonical parent, no link as the target). Nothing reaches the network.

## Resources (`resources.rs`)

- `caditor://model/summary` and `caditor://model/parameters` (JSON, the `get_model` and
  `list_parameters` answers) and `caditor://guide/<page id>` for every guide page, rendered from
  the parsed page to Markdown with commands as their titles and links as guide URIs.

## Tests

- `mcp/tests.rs` drives the server over in-memory pipes (`Client`, a thread serving
  `serve_watching`), and the cancellation test holds the output at the first progress
  notification until the cancel is registered (`Gate`), so it never races.
  `mcp/end_to_end_tests.rs` builds a plate with a pocket and two holes by tool calls alone and
  checks its volume, undo, redo, a parameter change, save, export and the saved file.
