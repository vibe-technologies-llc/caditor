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
  `FolderRef`, `Folder` (sub-folders and `Design`s), `Translation` and `Download`, whose strings
  only their provider reads. Ids are newtypes, never positional.
- `Provider` (`provider.rs`): `start_sign_in`/`finish_sign_in` (OAuth 2 with PKCE, `Redirect`,
  `Callback`, `SignIn`), `refresh`, `revoke`, `account`, `workspaces`, `projects`, `root`,
  `folder`, and the conversion trio `start_conversion`, `poll_conversion`, `fetch`. A service is
  one variant of `Service` (`service.rs`, which also holds the words that differ: `company`,
  `app_name`, `project_name`, `has_workspaces`) and one `Provider`: a new service is that variant, a
  module mapping its projects, workspaces and STEP translation onto this vocabulary, one arm in
  `Cloud::provider` and its copy in Home's `Wording`. `rest.rs` holds what Fusion and Onshape share:
  the status-to-`Error` mapping, JSON reading, the bearer `get` and the token exchange.
- `Transport` (`transport.rs`) is `send` (a bounded JSON answer) and `fetch` (a streamed body into a
  sink with progress and stop); its errors are `TransportError`s. `Network` (`network.rs`) is
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
- Import: the item's tip version, its id as an unpadded URL-safe base64 URN, a Model Derivative
  job to `step`, the manifest polled until the `step` derivative has a child with role `step`, then
  that derivative streamed. Fusion's own file format is closed, so STEP through Autodesk is the way
  a design comes in; it then goes through the ordinary STEP import.
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
- One synthetic `Workspace` ("Your documents", `Service::has_workspaces` false hides it in the UI).
  Projects are the account's documents from `/documents`, newest first, following `next` only inside
  `cad.onshape.com`, at most `MAX_PAGES` pages. A project's root is its default workspace's Part
  Studios and assemblies (`/documents/d/{did}/w/{wid}/elements`), no folders. A `DesignId` is
  `<partstudios|assemblies>/<workspace>/<element>`, since the translation endpoint needs all three.
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
- The session is `<state>/cloud/<service>-session.json` (`SessionStore`), written through
  `caditor_file::write_privately` (an owner-only file, new or replaced; a test holds the mode); a
  damaged file is dropped, not fatal. Sign-out and turning cloud off revoke at the service (best effort) and delete it. An
  OS keyring is a recorded follow-up.
- An import is one job (`ImportJob`): convert, poll every two seconds for ten minutes at most,
  stream to `<state>/cloud/downloads/<id hash>/<design name>.step` through a hidden `.part` file,
  fsync, rename, then keep the newest `KEPT_DESIGNS`. One runs at a time; Cancel stops it between
  and inside steps. The finished file reaches the model as `FileCommand::Drop`, so every import
  rule in `app-files.md` applies (report, placement, one change per body, Reload import from its
  file); a refused drop leaves the file where it is. A file is named after its design so the body is.
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
  first advert shown also holds Cloud projects… and Turn off cloud features. Signed in, a header with hub and account (Sign out, Cloud
  projects…, Refresh) and a line per project, `PROJECT_ROWS_ON_HOME` of them, each loading its root
  only once drawn (`LoadProject`), with the newest designs as tiles that import on a click and
  `N more…` going to the manager. Tiles are disabled while an import runs, and the import's
  progress, failure and Cancel sit at the head of the section.
- The manager (`Command::Cloud`, File › Cloud) is the window for everything outside Home:
  accounts (state pill, Sign in, Cancel, Sign out, Refresh, the last failure), then per signed-in
  service a hub combo (only where `Service::has_workspaces`), a project list beside a folder
  browser with breadcrumbs and an Import per design (`CloudDialog` keeps one browser per service), the downloaded
  designs (count, size, Clear), Show cloud projects on Home, and Turn off cloud features at the
  footer's far left. A cloud turned off shows one callout and Turn on cloud services.
- Shared parts are `cloud_parts.rs`: tiles on `raised` over the page's `panel`, relative times
  ("Edited 3 days ago"), SI sizes, the import status. Every control is a widget-kit one, named for
  screen readers, and every cloud action is a palette command (`Home`, `Cloud`, `CloudSignIn`,
  `CloudSignOut`, `OnshapeSignIn`, `OnshapeSignOut`, `CloudRefresh`, which refreshes every signed-in
  service) refused with a reason when it cannot run.

## Dependencies

`ureq` (`rustls-no-provider`, `rustls-webpki-roots`), `rustls-graviola` and `graviola` (SHA-256 and
the system random source for PKCE). `webpki-roots` is data under CDLA-Permissive-2.0, allowed for
that crate alone in `deny.toml`. No dependency builds C.

## Tests

`caditor-cloud` tests every provider call against Autodesk- and Onshape-shaped JSON through
`ScriptedTransport`, PKCE against RFC 7636, the listener over a real socket and the client's
refresh and retry. `ui_tests/home.rs` drives the app with a `FakeProvider` through a real loopback
sign-in: the page, its order, one-click import into a model, failure and Try again, restart,
sign-out, opt-out (no call reaches the provider), the manager and Preferences. The live handshake
with Autodesk's hosts is not part of any test; a run needs a registered app.
