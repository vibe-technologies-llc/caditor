# caditor roadmap

## Direction

caditor is a parametric CAD application for Linux. The project is judged on two things before
anything else:

1. **User experience.** Modelling should feel direct and predictable, without the failure modes
   that FreeCAD is known for: broken references after an upstream edit, workbench and mode
   juggling, opaque errors and a UI that freezes. The concrete requirements are in
   `.claude/rules/ux.md`.
2. **Never losing work.** Crashes and data loss are catastrophic failures. The concrete
   requirements are in `.claude/rules/reliability.md`.

Features are added only once they meet both bars; an unpolished feature is not shipped. An item
is implemented when it works end to end in the app, not when the APIs exist. Implemented items
and resolved decisions are removed from this file, and a milestone disappears once it is empty.
Git history is the record of what was done.

## Open decisions

These need an answer before the milestone that depends on them starts.

- [ ] **Distribution** (blocks M8): Flatpak, AUR and/or plain release binaries.

## M8: Release

- [ ] Packaging, per the open decision above
- [ ] Release process and changelog
