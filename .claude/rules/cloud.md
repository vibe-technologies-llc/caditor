---
paths:
  - "crates/caditor-cloud/**"
  - "crates/caditor/src/cloud.rs"
  - "crates/caditor/src/cloud/**"
  - "crates/caditor/src/cloud_dialog.rs"
  - "crates/caditor/src/cloud_parts.rs"
  - "crates/caditor/src/files/cloud_send.rs"
  - "crates/caditor/src/home.rs"
  - "crates/caditor/src/browser.rs"
  - "crates/caditor/src/ui_tests/home.rs"
  - "crates/caditor/src/ui_tests/opening_in_fusion.rs"
---

# Home and cloud services

## The user's choice

- Cloud is on by default, so Home can advertise it, and it is opt-out entirely. Turning it off
  (`cloud.enabled`; Home's Turn off cloud features, the manager's footer, Preferences › General ›
  Cloud) takes every cloud entry away: no advert or project on Home, no File › Cloud submenu, the
  palette commands refused with `Refusal::Off` naming how to turn it back on, no thread, no
  request. It also revokes and forgets a saved sign-in (`Cloud::configure`). `Section::Cloud` is not
  resettable, so Restore defaults never turns it back on.
- caditor contacts a service only after the user clicks Sign in to it; a saved sign-in makes requests
  from the next start on, nothing else does (`Cloud::start_restoring` reads one small file).
  `cloud.on_home` keeps the cloud off Home while it stays in the manager and the palette.
- Home itself is `onboarding.home` (Preferences › General › Starting and recent files, or Home's
  footer), read leniently, on by default.

## `caditor-cloud`

The abstraction layer, free of the app and of any UI. A service is a `Provider` (object safe) over
a `Transport`; the app never speaks a service's protocol itself.

- Vocabulary (`catalogue.rs`): `Account`, `Workspace` (a Fusion hub; `WorkspaceKind`), `Project`,
  `FolderRef`, `Folder` (sub-folders, `Design`s a service converts, and `Upload`s it stores as they
  are, each with an `UploadFormat` told from its file name: STEP, STL, OBJ, 3MF, DXF, SVG),
  `Version` (a design's saved version, `number` and time), `Translation` and `Download`, whose
  strings only their provider reads. A `Folder` also names its `place`, the `FolderRef` a file can be
  put in (a Fusion folder's own; none for a project top that lists several folders, and for an
  Onshape version, which cannot change; an Onshape workspace is its own place). `Outgoing` (folder, file name, bytes) is what is sent and `Sent` (the name, a
  `SendOutcome`, `Created` or `NewVersion { number }`, and what opening it needs: the `ItemId` and
  `VersionId` Fusion made or found and the https `web_link` its answer carried, each absent when the
  service does not say) what came of it. Ids are newtypes, never
  positional.
- `Provider` (`provider.rs`): `start_sign_in`/`finish_sign_in` (OAuth 2 with PKCE, `Redirect`,
  `Callback`, `SignIn`), `refresh`, `revoke`, `account`, `workspaces`, `projects`, `root`,
  `folder`, `versions` (a design's saved versions, newest first; none for a service whose versions
  are folders), the conversion trio `start_conversion` (the tip, or a chosen `Version`),
  `poll_conversion`, `fetch`, `fetch_upload` (an `Upload` streamed as it is stored, with no
  conversion), `send` (an `Outgoing` into a folder, with progress and stop),
  `send_part_studio`/`poll_part_studio` (a service that can import the file as its own model
  rather than store it: `NotOffered` by default; Onshape only) and
  `thumbnail`
  (a design's picture as the bytes the service sent, `Ok(None)` when it has none; the default is
  none, so a service without one implements nothing). A service is one variant of `Service`
  (`service.rs`, which also holds the words that differ: `company`, `app_name`, `project_name`,
  `has_workspaces`) and one `Provider`: a new service is that variant, a module
  mapping its projects, workspaces and STEP translation onto this vocabulary, one arm in
  `Cloud::provider` and its copy in Home's `Wording`. `rest.rs` holds what Fusion and Onshape share:
  the status-to-`Error` mapping, JSON reading, the bearer `get` and the token exchange.
- `Transport` (`transport.rs`) is `send` (a bounded answer body, JSON or a
  picture's bytes; a request is a GET, POST or PUT whose body is empty, a form, JSON, JSON:API
  (`application/vnd.api+json`), bytes or a `Multipart` of text fields and one file, whose boundary
  is random per upload and whose names reach the headers percent-encoded unless plain ASCII),
  `send_tracked` (the same for a multipart, reporting the body's bytes as they go out and stopping
  when progress says so; `Network` streams it, other bodies fall back to `send`) and `fetch` (a streamed body into a sink with progress and stop); its errors are `TransportError`s. `Network` (`network.rs`) is
  ureq over rustls with the pure-Rust graviola crypto provider and bundled roots: no OpenSSL, no
  `ring`, no C. HTTPS only, at most five redirects, an answer capped at 16 MiB, separate timeouts
  for answers, downloads and uploads (a request carrying bytes or a multipart, which also wait
  for the body to go out), a `caditor/<version>` User-Agent and nothing else identifying.
- `Client` (`client.rs`) holds a provider, a transport and the `Session`: it refreshes an
  expiring session once under a lock (so two jobs never spend one refresh token twice), retries a
  call once after a 401 and then says `Error::SignedOut`, and hands each renewed session to a
  `Saved` callback. `finish_sign_in` refuses a callback whose `state` is not the attempt's.
- `Session` (`session.rs`) and `Verifier` (`pkce.rs`) never print a secret, not even in `Debug`.
- `Listener` (`loopback.rs`) listens on 127.0.0.1 only, for one `GET /callback?code=…&state=…`,
  turns away every other request, honours a stop flag and a deadline fixed before its loop, and
  answers the browser with a static page that holds nothing from the request.
- Errors are `Error` (typed; Display is words that say what to do next, never internals).
  `fake.rs` (feature `test-support`) holds `ScriptedTransport` and `FakeProvider` for the tests of
  this crate and the app; the provider can hold sends until released and answer without web links.

## Fusion 360 (`fusion.rs`)

Autodesk Platform Services. A public client: OAuth v2 authorisation code with PKCE (S256), no
client secret, scopes `data:read data:write data:create user-profile:read` (write is what asking
Autodesk to convert a design needs, create and write what sending a model needs; caditor never
changes a design). A refresh names no `scope`, so it keeps the grant's own, which is how a sign-in
made before `data:create` was added goes on working for everything but sending (Autodesk answers
403, worded as `Error::SendForbidden`, which says to ask for edit access or sign in again).

- Account: the OIDC `userinfo`. Hubs, projects and folders: Data Management (`/project/v1`,
  `/data/v1`), following `links.next` only inside `developer.api.autodesk.com`, at most
  `MAX_PAGES`. A project with one top folder shows its contents, with several the folders. Only
  `items:autodesk.fusion360:Design` items are listed.
- Import: the item's tip version (or the version the user chose from `…/items/{id}/versions`, whose
  `versionNumber` and `lastModifiedTime` make a `Version`), its id as an unpadded URL-safe base64
  URN, a Model Derivative job to `step`, the manifest polled until the `step` derivative has a child with role `step`, then
  that derivative streamed. The design's picture is the Model Derivative thumbnail
  (`<urn>/thumbnail?width=200&height=200`, a PNG) of the same version URN; 404 and 202 (nothing
  rendered yet) are no picture, not errors. Fusion's own file format is closed, so STEP through
  Autodesk is the way a design comes in; it then goes through the ordinary STEP import.
- Uploaded files: an `items` entry that is not a Fusion design and whose display name ends in a
  known `UploadFormat` extension (`Resource::entry`; hidden items and other kinds such as PDF are
  left out) is listed as an `Upload` beside the designs. `fetch_upload` asks the item's tip for
  `relationships.storage.data.id` (`urn:adsk.objects:os.object:<bucket>/<object>`), asks Object
  Storage for a signed S3 link (`/oss/v2/buckets/{bucket}/objects/{object}/signeds3download`, ten
  minutes) with the bearer, and streams that link without the bearer. A missing or malformed
  storage object, or a link absent or not https, is `Error::NoStoredFile`. No Model Derivative job
  is made.
- Sending (`Provider::send`) follows Autodesk's Data Management upload flow for a file that is not
  a Fusion design (`items:autodesk.core:File`), in order: the folder's contents are read for an item
  of the same display name (case-insensitive, a Fusion design never counts); `POST
  /data/v1/projects/{id}/storage` (JSON:API, the folder as `target`) makes the storage object
  `urn:adsk.objects:os.object:<bucket>/<object>`;
  `GET /oss/v2/buckets/{bucket}/objects/{object}/signeds3upload?parts=N&firstPart=1&minutesExpiration=60`
  gives an `uploadKey` and N https links; each part (`PART_BYTES`, 5 MiB, the last may be
  smaller) is PUT to its link without the bearer, with progress reported before the first part and
  after each; `POST` to the same `signeds3upload` with `{"uploadKey"}` completes the object; then
  `POST …/items` with the first version (`included`, storage as its relationship) makes the file,
  or `POST …/versions` with the existing `item` and the storage makes a new version of it. A 403
  is `SendForbidden`, a 409 `NameTaken` (a name Autodesk will not reuse), a part the storage
  refuses or a completion that fails `UploadIncomplete`, a link count that is not the part count or
  a non-https link `Unreadable`. The answer of the item or version `POST` is read for the `Sent`:
  the created item's `data.id` with the first `included` version, or the found item's id with the
  new version's `data.id`, and `links.webView.href` of the item, else of the version, kept only
  when it is https and holds no whitespace. Onshape fills none of these. Progress returning false stops it with `Error::Stopped` before
  the completion, so no item is made and the storage object it leaves behind is never listed. The
  flow is tested only against the documented JSON shapes, like the rest of this module.
- The app registered with Autodesk is a deployment fact, not code: a public-client app with the
  Data Management and Model Derivative APIs and the callback
  `http://localhost:49617/callback` (`CALLBACK_PORT`). Its client ID reaches a build as
  `CADITOR_AUTODESK_CLIENT_ID` (the release workflow) or `DEFAULT_AUTODESK_CLIENT_ID`
  (`cloud::built_in_client_id`, empty until the project's app is registered) or a user types it in
  Preferences (`cloud.fusion.client_id`, which wins). Without any, Sign in is disabled and says
  so, naming Preferences. Team hubs may need their administrator to allow the app; a refused hub
  says so (`Error::Forbidden`).

## Onshape (`onshape.rs`)

A public client by intent: OAuth 2 authorisation code with PKCE (S256), no client secret and no
`scope` parameter (an Onshape application's permissions are set at registration), against
`oauth.onshape.com`. Onshape's documentation shows only confidential clients and never mentions
PKCE, so whether its server takes this has not been verified (`docs/DECISIONS.md`).

- Account: `users/sessioninfo` (name, else email). Revoking is not sent: Onshape documents none, and
  the user ends a grant in their Onshape account, so sign-out only forgets the session.
- `Workspace`s are the views of the documents (`Service::has_workspaces`, the UI calls them a
  "view" by `Service::workspace_noun`), `onshape/scope.rs` `Scope`: My documents (`filter=0`, the
  first, `WorkspaceKind::Personal`), Recently opened (5, in Onshape's own order), Shared with me
  (2), one per company from `/companies` (7, `owner`, `ownerType=1`), one per team from `/teams`
  (9, `owner`) and Public documents (4). A refused or unreadable company or team list leaves
  those views out; a lost sign-in or a rate limit still fails. Projects are the view's documents
  from `/documents`, newest first except Recently opened, following `next` only inside
  `https://cad.onshape.com/api/`, at most `MAX_PAGES` pages; only `jsonType` document nodes
  (absent, `document`, `document-summary`) count, so a folder, label or project node in a listing is
  never a document.
- A project's root is its default workspace's Part Studios and assemblies
  (`/documents/d/{did}/w/{wid}/elements`), then sub-folders for its other branches
  (`/documents/d/{did}/workspaces`, "Branch X", by name) and its versions
  (`/documents/d/{did}/versions?limit=50`, "Version X", newest `createdAt` first). A
  `FolderId` is `w/<id>` or `v/<id>` (`Place`), and `folder` lists that place's elements
  (`.../{w|v}/{id}/elements`), no deeper. A failing branch or version list leaves them out of the
  root, never the designs. A `DesignId` is `<partstudios|assemblies>/<w|v>/<id>/<element>`,
  since the translation endpoint needs all of them. Drawings and Part Studio parts are
  not offered; a blob element whose name ends in a known `UploadFormat` extension is an `Upload`
  (`w|v/<id>/<element>`, `UploadPlace`), listed beside the designs. An element carries no date, so
  a workspace's blobs are dated from `/documents/d/{did}/w/{wid}/documenthistory` (read only when
  the workspace holds a blob of a known format): the entry whose `microversionId` is the element's
  own gives `date`. A history that cannot be read, or has no such entry, leaves the blob undated;
  only a lost sign-in or a rate limit fails the folder. A version's blobs are never dated. That the
  element's `microversionId` matches a history entry is unverified against a live server
  (`docs/DECISIONS.md`).
- Import: a `translations` request with `formatName: STEP` and `storeInDocument: false` (the
  document is never changed), `Translation::source` being `<document>/<translation>`, polled until
  `requestState` is `DONE` with a `resultExternalDataIds` entry, then
  `/documents/d/{did}/externaldata/{id}` streamed.
- The client ID reaches a build as `CADITOR_ONSHAPE_CLIENT_ID` or `DEFAULT_ONSHAPE_CLIENT_ID`, or a
  user types it in Preferences (`cloud.onshape.client_id`, which wins), as for Autodesk.
- Uploaded files: `fetch_upload` streams `/blobelements/d/{did}/{w|v}/{id}/e/{element}` with the
  bearer.
- Sending (`Provider::send`) puts the STEP into a document's workspace as a blob element. The root
  and a branch carry a `place` (`w/<id>`), a version does not (`Error::VersionIsReadOnly` if one is
  sent to anyway). In order: the workspace's elements are read for a `BLOB` of the same name
  (trimmed, case-insensitive; a Part Studio of that name never counts); then one multipart `POST
  /api/v10/blobelements/d/{did}/w/{wid}` (a new element) or `…/e/{eid}` (updating the found blob,
  which Onshape keeps as a new microversion in its history, so `SendOutcome::NewVersion` has no
  number) with the file part, `encodedFilename` (the name percent-encoded), `translate=false` and
  `storeInDocument=true`, so Onshape stores the file as it is and never translates it into a Part
  Studio. 403 is `SendForbidden`, 401 `SignedOut`. The body goes out through
  `Transport::send_tracked`, whose `Network` implementation hands the encoded multipart to ureq as a
  reader with an explicit `Content-Length` and reports each chunk handed over (`Tracked`, at most
  64 KiB at a time); progress returning false ends the read, the transfer is dropped and the result
  is `TransportError::Stopped`, so a large file can be cancelled mid-send and nothing is created.
  The provider reports the file's own bytes (the body's few hundred bytes of framing are capped
  away), and the full size again once the answer is read.
- Sending as a Part Studio (`Provider::send_part_studio`, `poll_part_studio`, `SendAs::PartStudio`;
  `Service::sends_part_studios` gates the manager's Send as a Part Studio button): the same
  multipart `POST /blobelements/d/{did}/w/{wid}` with `translate=true`, `formatName=STEP`,
  `storeInDocument=false` and `encodedFilename`, no existing element involved (each send makes a new
  Part Studio). The answer is a translation request (`id`, `requestState`, with `resultElementIds`
  and `resultExternalDataIds` null until done, which `TranslationState` reads as empty), returned as
  a `Translation` (`<document>/<id>`). `poll_part_studio` reads `/translations/{id}`: `DONE` with a
  `resultElementIds` entry is `Placement::Done`; `FAILED`, or `DONE` with none, is
  `Error::PartStudioFailed`; anything else is `Placement::Working`. Onshape's published API has no
  way to cancel a translation, so Cancel only stops waiting.

## The app service (`cloud.rs`)

- `Cloud` lives in `Workspace`, one `Connection` per `Service` (Fusion 360, Onshape) with a typed `Link`: `Off`,
  `Starting`, `SignedOut`, `SigningIn`, `Linked`. `Linked` holds `Load<T>`s (`Idle`, `Loading`,
  `Ready`, `Failed`) for the account, hubs, projects and each folder, plus the running `Import`.
  The UI reads `&Cloud` and returns `CloudCommand`s as `Action::Cloud`; `app::perform` runs them
  and the actions they answer with, and `app::show` polls it each frame.
- Every request is a thread of its own (`Courier::spawn`, named `cloud`) under `catch_unwind`; the
  answer is an `Event` carrying the connection's generation, which a sign-out, refresh or hub
  change bumps, so a late answer is dropped. A thread that cannot start is a `Failure::Spawn`
  answer, never a request on the UI thread. Each frame the UI thread only drains a channel.
- Sign-in: bind the listener, `start_sign_in`, publish the URL (Home and the manager show Copy the
  sign-in link, since a browser may not open), open it with `browser::open` (https only, no shell:
  `xdg-open` or `gio`, `rundll32 url.dll,FileProtocolHandler`), wait up to five minutes, trade the
  code, save the session. Cancel stops the wait.
- The hub a user chooses is remembered in `<state>/cloud/<service>-hub.json` (written atomically
  by a job on the courier, read by the job that lists the hubs). After a restart or Refresh the
  hub in use wins, then the remembered one if it is still listed, then `first_workspace` (the team
  hub first); a vanished hub keeps its file, so it returns with the hub. Only a deliberate choice writes it (`ChooseWorkspace`, never the automatic pick); an
  explicit sign-out or turning cloud off deletes it with the session, an ended sign-in does not.
- Upload to Fusion 360 and open online (`CloudOpenInFusion`, `CloudCommand::OpenInFusion`; File › Cloud,
  File › Export and the palette, gated by `Cloud::can_open_in_fusion` and
  `Files::cloud_send_availability`, so a refusal names its reason before anything is exported) is
  the ordinary send with a `Destination` whose `after` is `AfterSend::OpenOnTheWeb`. The folder is
  remembered per hub, in memory in `Linked::destinations` and in
  `<state>/cloud/fusion-destination.json` (`SessionStore`, a list of hub, project, folder and
  folder name written atomically by a courier job, each write carrying a sequence so a late older
  write is skipped, read by the job that lists the hubs and never overriding what this run already
  holds). It is remembered when a send with `after` starts from the manager's Send and open in
  Fusion 360 button (primary when the dialog was opened by the command with nothing remembered;
  the manager says which folder the command sends to). With a folder remembered for the hub in
  use the command runs `FileCommand::SendToCloud` at once; with none it opens the manager
  (`ShowCloudToOpen`). The decision to keep it per hub in the state directory, not in the model
  file or the preferences, is recorded as open in `docs/DECISIONS.md`. Sign-out and turning cloud
  off delete the file with the hub's.
- The Export dialog has the command as an Upload to Fusion 360 button beside Open in OrcaSlicer
  (`ExportCommand::OpenInFusion`, `export::FUSION_LABEL`). `Files` cannot start a cloud send, so
  `files::show` turns the outcome into `Export(Hide)` and `CloudCommand::OpenInFusion` and nothing
  reaches `Files::perform`. `app::show` hands it the command's own verdict (`Cloud::can_open_in_fusion`
  then `Files::cloud_send_availability`): no button with cloud off, the button disabled with the
  reason (or the dialog's own blocker) otherwise.
- A send that opens (`Destination::opening`) of a model with no file asks for the name first, as
  nothing else names it: `Files::send_to_cloud` checks `cloud_send_availability` and then holds a
  `CloudNaming` (`files/cloud_send.rs`; blocking like the other file dialogs) shown as the titled
  dialog "Name the file for Fusion 360" with one File name field, prefilled with the name the
  last such send of this model used (`CloudSending::named`, keyed by the model's session) or
  Untitled, and Send and open rightmost. `CloudNameCommand::{Edit, Send, Cancel}` drive it; Enter
  sends. The name is trimmed, a trailing `.step` in any case is dropped and `.step` appended
  (`NameRefusal::Empty`, `UnsafeCharacter` refuse in a callout and disable Send). It travels as
  `Stage::Preparing`'s and `Event::CloudSendPrepared`'s `name` into `cloud_send_prepared`, which
  uses it for the file in place of `export::named_for`; a model that was replaced meanwhile sends
  nothing and says so. A saved model, and a plain Send the model here, keep the model's own name
  and never ask.
- Opening is the web page only, on every platform: after `Delivered` carries a `web_link`, the
  courier runs the injectable `Opener` (`browser::open`, https only, no shell) in a job, never on
  the UI thread, so the upload is never undone by a failure to open. The success notice says the
  file opens in Fusion as an import (unconfirmed against a live Fusion) or, with no `web_link`,
  that Autodesk gave no address. A failed open (`Failure::Browser`) is a notice and
  `Linked::unopened`, which the manager shows at the head of the Fusion section with Copy link and
  Dismiss until the next send or Dismiss. A send with `after` that fails with a missing folder or
  `SendForbidden` forgets the remembered folder, says so naming the folder and the file, and opens
  the manager to choose another; any other failure stays `SendStage::Failed` with Try again, as a
  plain send. Fusion's desktop URL handler is not used (`docs/FEATURES.md`).
- The session is JSON kept by `SessionStore` (`cloud/store.rs`) in the operating system's keyring
  (`cloud/keyring.rs`, below), one entry per service, and in `<state>/cloud/<service>-session.json`
  (`caditor_file::write_privately`, an owner-only file, new or replaced; a test holds the mode) only
  when no keyring takes it. `save` writes the keyring and then removes any file; when the keyring
  fails it writes the file. `load` reads a file first, since a file is by construction newer than
  any keyring entry (it exists only because a keyring write failed, and a refresh token is spent
  once): it moves the file into the keyring and deletes it on success, and keeps it while the keyring
  cannot take it. With no file it reads the keyring; an unreachable keyring is no sign-in, never an
  error. A damaged file or entry is dropped, not fatal. `forget` removes both; a keyring that is
  absent (no bus, no Secret Service, no default collection) is not an error, one that refuses
  (locked and not unlocked, prompt dismissed) is logged, since the entry would sign the user in
  again at the next start. Sign-out and turning cloud off revoke at the service (best effort) and
  forget; every keyring call, forgetting included, runs on a courier thread, never on the UI
  thread (`Cloud::leave` spawns even when nothing is linked).
- An import is one job (`ImportJob`) of an `ImportItem`: a `Design` (the tip), a `Version` of a
  design, or an `Upload`. A design or version is converted, polled every two seconds for ten
  minutes at most, and streamed; an upload is streamed straight (`Client::fetch_upload`, progress
  `Stage::Downloading` only). Either goes to `<state>/cloud/downloads/<id hash>/<name>.<ext>`
  through a hidden `.part` file, fsync, rename, then the newest `KEPT_DESIGNS` are kept (`.step`
  for a design, `Name vN.step` for a version, the upload's own name with its format's extension).
  The hash covers project, item id and, for a version, the version id, so a version never replaces
  the tip's file. One runs at a time; Cancel stops it between and inside steps. The finished file
  reaches the model as `FileCommand::Drop`, so every import rule in `app-files.md` applies (the
  extension picks STEP, mesh or drawing import; report, placement, one change per body, Reload
  import from its file); a refused drop leaves the file where it is. A file is named after its
  item so the body is.
- Sending is one job (`SendJob`) of a `Shipment` (a `Destination` of service, folder and the
  folder's name for words, the file name and the exported STEP file), run at most one at a time per
  service (`Linked::sending`, `Send` with `SendStage`: `Starting`, `Uploading`, `Translating`,
  `Failed`). The job reads the file, `Client::send`s it with progress throttled like a download's,
  and removes the file whatever came of it; Cancel stops it between parts, and for Onshape inside
  the request body. A `Destination` carries its `SendAs`: `File` is the above, `PartStudio`
  (Onshape's Send as a Part Studio button) `Client::send_part_studio`s and then polls
  `poll_part_studio` every two seconds for ten minutes at most (`Error::PartStudioTimedOut`),
  reporting `Translating`, Cancel stopping the wait between polls; the outcome is
  `SendOutcome::PartStudio`, worded as a new Part Studio. Try again keeps the `SendAs`. On success the destination folder and the
  project's root are dropped from `Linked` so the manager lists the new file, and a notice says
  whether it is a new file or version N; a sign-in that has ended signs the connection out with a
  notice; any other failure stays as `SendStage::Failed` with Try again, which exports again. The
  model reaches the job through `files/cloud_send.rs`: `FileCommand::SendToCloud(Destination)` is
  refused as opening in the slicer is (`CloudSendRefusal`: no body, all left out, an export
  running, a recompute running, already preparing, no state folder), clears and makes
  `<state>/cloud/outgoing/` on a job, exports the bodies chosen for export as STEP there with the
  ordinary exporter (so its cancel, its left-out rules and its notices apply; a left-out body means
  nothing is sent), and hands the file to `app::perform` through `Files::take_cloud_shipment`,
  which runs `CloudCommand::Send`. `Cloud::can_send_anywhere` (cloud on, a signed-in service) gates the `CloudSend` command.
- A design's versions (`Linked::versions`, `Load<Vec<Version>>` per `DesignId`) load only when the
  user opens them in the manager (`LoadVersions`; `ReloadVersions` after a failure) and are
  dropped by Refresh and by choosing another hub.
- A design's picture (`cloud/thumbnails.rs`) is decoration, loaded lazily: Home asks
  (`LoadThumbnail`) for each design tile it draws inside the visible area, one courier job per
  design, at most `AT_ONCE` running (the rest ask again on the next frame), the answer a
  `Load<Option<Arc<Thumbnail>>>` in `Linked::thumbnails`. The job reads
  `<state>/cloud/thumbnails/<service>/<hash>.png` first (the hash covers service, project, design
  and modified time, so a new version is a new file), else asks the provider's `thumbnail`, refuses
  an answer over `MAX_THUMBNAIL_BYTES`, decodes it with `caditor_file::decode_picture` and keeps the
  bytes atomically, `KEPT_THUMBNAILS` newest. A missing or unreadable picture is `Ok(None)` and a
  failed request a `Load::Failed` that nothing shows: the tile keeps its icon and never a callout.
  The decoded `ColorImage` becomes a texture on the UI thread on first use (`Thumbnail::texture`).
  Refresh and choosing another hub drop the in-memory pictures; an explicit sign-out or turning
  cloud off deletes the folder, an ended sign-in does not. Samples keep icons; a recent file's tile
  shows the picture its model file holds (below).
- Failures are `Failure`s worded for the user; a 401 that survives a refresh signs the connection
  out with a notice, anything else is a callout with Try again on Home and in the manager.

## Home (`home.rs`) and the manager (`cloud_dialog.rs`)

- Home is a page in an `egui::Area` over the viewport's rect, opaque, scrolling, capped at 1040
  points. It opens at startup when caditor starts empty (`startup` is an empty model, no file on
  the command line, `onboarding.home`), from File › Home and the palette; it closes on Escape, its
  close button, any start, an opened file or sample, and whenever the model it opened over changes
  (`Home::follow`: session, revision, a sketch being edited or a plane chosen). While it is open
  the start card stays away (`ViewportState::cover`). Tests never open it by default.
- Recent files tiles show the model's own preview picture (`app-files.md`, Model previews): a tile
  in view with none known asks `Choice::LoadRecentPicture` (`FileCommand::LoadRecentPicture`, only
  while `Files::has_room_for_recent_picture`, so no more than `AT_ONCE` reads run), and draws
  `Files::recent_picture` through `cloud_parts::Tile::picture` once it arrives, the same widget the
  Fusion pictures use (`Thumbnail`); a file without a picture, a damaged or missing one keeps its
  icon and never shows a callout. The tile's picture box is `TILE_PICTURE_WIDTH` by
  `TILE_PICTURE_SIDE` and the picture is fitted into it, so a square Fusion picture and a 4:3
  preview both keep their proportions.
- Order: header, starts (New sketch primary, box, import, open), Recent files (the history, tiles
  from `Files::recent`), Fusion 360, Onshape, samples, footer. Signed out, a service is an advert
  card: what it does, three honest lines about what it sends, Sign in (or the no-app callout); the
  first advert shown also holds Cloud projects… and Turn off cloud features. Signed in, a header
  with hub and account (Sign out, Cloud projects…, Refresh) and a line per project,
  `PROJECT_ROWS_ON_HOME` of them, each loading its root only once drawn (`LoadProject`), with the
  newest designs as tiles (uploaded files are not shown on Home) that import on a click (the design's picture replaces the tile's icon once
  it arrives) and `N more…` going to the manager. Tiles are disabled while an import runs, and the
  import's progress, failure and Cancel sit at the head of the section.
- The manager (`Command::Cloud`, File › Cloud) is the window for everything outside Home:
  accounts (state pill, Sign in, Cancel, Sign out, Refresh, the last failure), then per signed-in
  service a hub combo (only where `Service::has_workspaces`), a project list beside a folder
  browser with breadcrumbs, an Import per design, a Versions toggle per design where
  `Service::has_design_versions` (rows of version number, time and Import, the newest marked) and
  the uploaded files after the designs with an icon for their kind (`cloud_parts::upload_icon`:
  a cube for STEP, a polygon for meshes, a vector drawing for DXF and SVG) and an Import
  (`CloudDialog` keeps one browser per service, with the designs whose versions are open), in
  a folder a Send the model here button (primary while the dialog
  was opened by the `CloudSend` command, which also shows a callout; disabled with the reason when
  the folder has no `place`, a send is running or `CloudSendRefusal` applies) with the send's
  progress, Cancel, failure and Try again at the head of the section (`cloud_parts::send_status`),
  a Cancel for the preparing export, the downloaded
  designs (count, size, Clear), Show cloud projects on Home, and Turn off cloud features at the
  footer's far left. A cloud turned off shows one callout and Turn on cloud services.
- Shared parts are `cloud_parts.rs`: tiles on `raised` over the page's `panel`, relative times
  ("Edited 3 days ago"), SI sizes, the import status. Every control is a widget-kit one, named for
  screen readers, and every cloud action is a palette command (`Home`, `Cloud`, `CloudSignIn`,
  `CloudSignOut`, `OnshapeSignIn`, `OnshapeSignOut`, `CloudRefresh`, which refreshes every signed-in
  service, `CloudSend`, which opens the manager to choose a folder, and `CloudOpenInFusion`; File ›
  Cloud lists both) refused with a reason when it cannot run.

## The keyring (`cloud/keyring.rs`)

`Keyring` (object safe: `get`, `set`, `delete` of a service's bytes, `KeyringError` an enum of
specific causes with `is_absent`) is the platform pair behind `keyring::system()`; `CloudServices`
carries it so tests inject `FakeKeyring` (reachable or not), and the UI tests of the file fallback
use an unreachable one. No test touches the real keyring.

- Linux (`keyring/secret_service.rs`): the freedesktop Secret Service over `zbus` (the workspace's
  zbus 6, already built for the portal), a new session-bus connection per call under `zbus::block_on`
  on the calling job thread. `OpenSession` with the `plain` algorithm, the default collection
  (`ReadAlias("default")`), items found by the attributes `application=caditor`, `kind=cloud-session`,
  `service=<id>`; `CreateItem` replaces the entry. A locked collection is unlocked (`Unlock` and its
  prompt, waited for at most two minutes and then dismissed by a helper thread, so a job never
  waits forever); a prompt the user dismisses is `PromptDismissed` and the file is used.
- `plain` is chosen over the DH session (`dh-ietf1024-sha256-aes128-cbc-pkcs7`) deliberately. The
  secret then crosses only the user's own session bus, a Unix socket in the user's private runtime
  directory, to a daemon that holds it as the user; a process of that user could equally read the
  owner-only file the keyring replaces, and root can do anything. The DH exchange would protect
  against a bus-wide eavesdropper that the bus's own permissions already exclude, and it would add
  a 1024-bit Diffie-Hellman, HKDF and AES-CBC, none of which `zbus` or the dependency tree supplies
  in pure Rust, for no gain here.
- Windows (`keyring/credential_manager.rs`): Credential Manager generic credentials through
  `caditor_windows::{read,write,delete}_credential` (`windows.md`), persisted for the local machine
  and encrypted by Windows for the user. A credential's blob is at most 2560 bytes and an access
  token can approach that alone, so the JSON is split into pieces `caditor/<service>/<n>` of at most
  that size; `set` writes them all and deletes leftover higher pieces, `get` reads from 0 until one
  is missing, `delete` removes them all. A torn write reads as damaged JSON and is dropped.

## Dependencies

`ureq` (`rustls-no-provider`, `rustls-webpki-roots`), `rustls-graviola` and `graviola` (SHA-256 and
the system random source for PKCE). `webpki-roots` is data under CDLA-Permissive-2.0, allowed for
that crate alone in `deny.toml`. `zbus` (Unix) and `caditor-windows` (Windows) serve the keyring. No
dependency builds C.

## Tests

`caditor-cloud` tests every provider call against Autodesk- and Onshape-shaped JSON through
`ScriptedTransport`, PKCE against RFC 7636, the listener over a real socket and the client's
refresh and retry. `ui_tests/opening_in_fusion.rs` drives the open command with a `FakeProvider` and an injected
`Opener` that records addresses, so no test starts a browser: the first run choosing a folder, the
second making version 2 without asking, Cancel, a browser that fails, an answer without an address,
a vanished folder, sign-out and the refusals. `ui_tests/home.rs` drives the app with a
`FakeProvider` through a real loopback sign-in: the page, its order, one-click import into a model, failure and Try again, sending a model
(a new file, then a new version, into an Onshape workspace, nothing sent without a body or from a project top, a refusal and
Try again, the palette command, opt-out), restart, sign-out, opt-out (no call reaches the
provider), the manager and Preferences. The live handshake
with Autodesk's hosts is not part of any test; a run needs a registered app.
