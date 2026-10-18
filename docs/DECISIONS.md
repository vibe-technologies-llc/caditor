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
  element named after the file, and that the same call on `/e/{eid}` updates it.
- [medium · medium] In-app assistant connection (`FEATURES.md`, Application): which transport the
  running app listens on for MCP clients (a local socket or named pipe per user, or streamable
  HTTP on localhost with a token), whether connecting needs a click in the app every time or once
  per client, whether an assistant's changes are one undo step per call or grouped per request,
  and whether it may save over the user's file or only export beside it.
- [low · medium] A body made of one separate piece of a feature (`app-modelling.md`, Separate
  pieces) takes its ID from the range of feature IDs no feature can hold, so body references keep
  their type through the code and the file format; the price is that a body ID is no longer always
  a feature ID. Decide whether to keep that or give bodies an ID type of their own, which reaches
  about 600 sites and the file format.
- [low · easy] Trim cuts at the reference axes as at any curve (`Cutter::Axis`), so a circle
  centred on the X axis, as a lever's pivot on the origin or a slot along the axis is, loses only
  the quarter between a tangent line's touching point and the axis when its inside is picked: the
  first click leaves an arc ending on the axis, held there by a `Coincident` with it, and a second
  click takes the rest. The same slot drawn off the axis trims in one click per end with every end
  joined to its line. Open decision for `trim.rs`: whether an axis should cut only a curve that no
  other curve cuts, or never.
- [low · medium] Open decision on kept measurements: a failed or suppressed measurement fails the
  features using its value, as a failing feature's dependents do; whether they should instead keep
  its last reading (the measured parameter's stored value already holds it) is undecided.
