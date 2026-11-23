# caditor decisions

Open decisions: large directions the project has not committed to, and smaller questions an
implementer cannot settle alone. Each is recorded in the rules file or `docs/` that covers it once
decided, and removed from here.

Entries are tagged and ordered as `ROADMAP.md` describes.

## Scope decisions

These are open: each is a large direction the project has not committed to, and each needs a
decision recorded in `docs/` before work starts.

- [high · hard · blocked by: a scope decision recorded in `docs/`] Assemblies: a model is one part
  of several bodies, with no components, instances of another model file, joints or mates, exploded
  views or bill of materials, so a product of several parts cannot be put together or checked for
  fit. Decide whether caditor stays a part modeller, or how assemblies reference part files while
  keeping references stable across edits. The same decision covers deriving: bringing the bodies,
  sketches or parameters of another model file into this one, linked so they update when that file
  changes, which a skeleton model driving several parts, or a part fitted to its neighbour, needs.
- [medium · hard · blocked by: a scope decision recorded in `docs/`] Surface modelling: no surface
  bodies, so no thicken, offset surface, trim, extend, patch or knit to a solid, which shaped
  consumer parts and repairing open STEP imports need. Also stitch and unstitch, boundary fill
  (a solid from the cell several surfaces and bodies enclose), ruled and sweep or loft surfaces,
  and freeform (T-spline) shaping, which Fusion keeps in its own environment.
- [low · hard · blocked by: a scope decision recorded in `docs/`] Sheet metal: no flanges, bends
  with a bend allowance, or flat patterns, though laser-cut and bent parts are a common use; flat
  patterns would go out through the DXF export of a flat face.

## Open questions

- [high · easy] The Fusion 360 and Onshape imports have no app behind them yet: the built-in client
  IDs (`DEFAULT_AUTODESK_CLIENT_ID` and `DEFAULT_ONSHAPE_CLIENT_ID` in `cloud.rs`, overridden at
  build time by `CADITOR_AUTODESK_CLIENT_ID` and `CADITOR_ONSHAPE_CLIENT_ID`) are empty, so Sign in
  is disabled until someone registers an Autodesk Platform Services app and an Onshape OAuth
  application for the project (`RELEASING.md`) and fills them in, or a user types their own in
  Preferences. Decide who registers them and under which accounts, whether the fixed callback port
  49617 is acceptable, and whether the team-hub administrator step (`.claude/rules/cloud.md`) needs
  a page of its own in the guide.
- [high · medium] Onshape's OAuth documentation shows only confidential clients (a `client_secret`
  on the token and refresh requests) and does not mention PKCE, and no live handshake has been run:
  `onshape.rs` signs in with PKCE and no secret, as Fusion does. Register the application, try it,
  and if Onshape refuses a secretless exchange decide between API keys typed in Preferences and a
  small token-exchange service; a secret is never shipped in the binary. The same run should
  confirm sending (`cloud.md`): that `POST /blobelements/d/{did}/w/{wid}` with `file`,
  `encodedFilename`, `translate=false` and `storeInDocument=true` stores the STEP file as a blob
  element named after the file, and that the same call on `/e/{eid}` updates it. It should also
  confirm the Part Studio send (`translate=true`, `formatName=STEP`, `storeInDocument=false` makes
  a Part Studio whose id the polled translation lists in `resultElementIds`), and that a blob's
  `microversionId` in an elements listing matches the `microversionId` of an entry of
  `/documents/d/{did}/w/{wid}/documenthistory`, whose `date` the manager shows for it (Onshape's
  published element record has no date of its own). It should also confirm that `/parts/d/{did}/…/e/{eid}`
  lists a Part Studio's parts with `partId` and `name`, and that a translation request with
  `partIds` set to one part id (a string) exports only that part.
- [medium · easy] Upload to Fusion 360 and open online (`.claude/rules/cloud.md`) remembers its folder per
  hub in `<state>/cloud/fusion-destination.json` and offers Send and open only in the folder the
  manager is showing. Decide whether the remembered destination belongs in the model file instead
  (a model sent to one project keeps going there, and travels with the file) or in the
  preferences, and whether to also offer a Send and open that asks for the folder every time. The
  same live-account run as the Onshape one above should confirm what the command assumes of
  Autodesk: `links.webView` on a created file item and version, and that opening an uploaded STEP
  item in Fusion imports it as a design (`FEATURES.md`, Application).
- [low · medium] A body made of one separate piece of a feature (`app-modelling.md`, Separate
  pieces) takes its ID from the range of feature IDs no feature can hold, so body references keep
  their type through the code and the file format; the price is that a body ID is no longer always
  a feature ID. Decide whether to keep that or give bodies an ID type of their own, which reaches
  about 600 sites and the file format.
- [low · easy] A decimal comma inside a call reads as an argument separator: typed `round(2,5)` is
  rounding 2 to steps of 5 (0) and `max(1,5)` is 5, while a top-level `2,5` is refused with the
  decimal-comma hint (`parse.rs`, `decimal_comma`) and `max(0,5)` is tested to parse. Decide
  whether typed text with a comma between digits and no space inside parentheses is refused as
  `DecimalComma`; stored text always writes `, `, so files are unaffected.
- [low · medium] Open decision on kept measurements: a failed or suppressed measurement fails the
  features using its value, as a failing feature's dependents do; whether they should instead keep
  its last reading (the measured parameter's stored value already holds it) is undecided.
