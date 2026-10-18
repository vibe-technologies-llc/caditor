---
paths:
  - "crates/caditor-cloud/**"
  - "crates/caditor/src/cloud.rs"
  - "crates/caditor/src/cloud/**"
  - "crates/caditor/src/cloud_dialog.rs"
  - "crates/caditor/src/cloud_parts.rs"
  - "crates/caditor/src/home.rs"
  - "crates/caditor/src/browser.rs"
  - "crates/caditor/src/ui_tests/home.rs"
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
  strings only their provider reads. Ids are newtypes, never positional.
- `Provider` (`provider.rs`): `start_sign_in`/`finish_sign_in` (OAuth 2 with PKCE, `Redirect`,
  `Callback`, `SignIn`), `refresh`, `revoke`, `account`, `workspaces`, `projects`, `root`,
  `folder`, `versions` (a design's saved versions, newest first; none for a service whose versions
  are folders), the conversion trio `start_conversion` (the tip, or a chosen `Version`),
  `poll_conversion`, `fetch`, `fetch_upload` (an `Upload` streamed as it is stored, with no
  conversion) and `thumbnail`
  (a design's picture as the bytes the service sent, `Ok(None)` when it has none; the default is
  none, so a service without one implements nothing). A service is one variant of `Service`
  (`service.rs`, which also holds the words that differ: `company`, `app_name`, `project_name`,
  `has_workspaces`) and one `Provider`: a new service is that variant, a module
  mapping its projects, workspaces and STEP translation onto this vocabulary, one arm in
  `Cloud::provider` and its copy in Home's `Wording`. `rest.rs` holds what Fusion and Onshape share:
  the status-to-`Error` mapping, JSON reading, the bearer `get` and the token exchange.
- `Transport` (`transport.rs`) is `send` (a bounded answer body, JSON or a
  picture's bytes) and `fetch` (a streamed body into a sink with progress and stop); its errors are `TransportError`s. `Network` (`network.rs`) is
  ureq over rustls with the pure-Rust graviola crypto provider and bundled roots: no OpenSSL, no
  `ring`, no C. HTTPS only, at most five redirects, an answer capped at 16 MiB, separate timeouts
  for answers and downloads, a `caditor/<version>` User-Agent and nothing else identifying.
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
  this crate and the app.

## Fusion 360 (`fusion.rs`)

Autodesk Platform Services. A public client: OAuth v2 authorisation code with PKCE (S256), no
client secret, scopes `data:read data:write user-profile:read` (write is what asking Autodesk to
convert a design needs; caditor never changes a design).

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
  since the translation endpoint needs all of them. Drawings, blobs and Part Studio parts are
  not offered.
- Import: a `translations` request with `formatName: STEP` and `storeInDocument: false` (the
  document is never changed), `Translation::source` being `<document>/<translation>`, polled until
  `requestState` is `DONE` with a `resultExternalDataIds` entry, then
  `/documents/d/{did}/externaldata/{id}` streamed.
- The client ID reaches a build as `CADITOR_ONSHAPE_CLIENT_ID` or `DEFAULT_ONSHAPE_CLIENT_ID`, or a
  user types it in Preferences (`cloud.onshape.client_id`, which wins), as for Autodesk.

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
  cloud off deletes the folder, an ended sign-in does not. Recent files and samples keep icons: the
  model file stores no preview (`docs/FEATURES.md`).
- Failures are `Failure`s worded for the user; a 401 that survives a refresh signs the connection
  out with a notice, anything else is a callout with Try again on Home and in the manager.

## Home (`home.rs`) and the manager (`cloud_dialog.rs`)

- Home is a page in an `egui::Area` over the viewport's rect, opaque, scrolling, capped at 1040
  points. It opens at startup when caditor starts empty (`startup` is an empty model, no file on
  the command line, `onboarding.home`), from File › Home and the palette; it closes on Escape, its
  close button, any start, an opened file or sample, and whenever the model it opened over changes
  (`Home::follow`: session, revision, a sketch being edited or a plane chosen). While it is open
  the start card stays away (`ViewportState::cover`). Tests never open it by default.
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
  (`CloudDialog` keeps one browser per service, with the designs whose versions are open), the downloaded
  designs (count, size, Clear), Show cloud projects on Home, and Turn off cloud features at the
  footer's far left. A cloud turned off shows one callout and Turn on cloud services.
- Shared parts are `cloud_parts.rs`: tiles on `raised` over the page's `panel`, relative times
  ("Edited 3 days ago"), SI sizes, the import status. Every control is a widget-kit one, named for
  screen readers, and every cloud action is a palette command (`Home`, `Cloud`, `CloudSignIn`,
  `CloudSignOut`, `OnshapeSignIn`, `OnshapeSignOut`, `CloudRefresh`, which refreshes every signed-in
  service) refused with a reason when it cannot run.

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
refresh and retry. `ui_tests/home.rs` drives the app with a `FakeProvider` through a real loopback
sign-in: the page, its order, one-click import into a model, failure and Try again, restart,
sign-out, opt-out (no call reaches the provider), the manager and Preferences. The live handshake
with Autodesk's hosts is not part of any test; a run needs a registered app.
