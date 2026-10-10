# caditor roadmap

This file is the index of what remains. The entries themselves live in the files below, one
subject each; check them before starting new work.

| File | Covers |
| --- | --- |
| [`FEATURES.md`](FEATURES.md) | Missing tools, feature options and exchange formats: modelling, sketching, STEP, drawings, meshes, technical drawings, the application. |
| [`BUGS.md`](BUGS.md) | Wrong results, silent losses and refusals of what should work today: files and recovery, kernel, modelling, the application, sketching, the solver, STEP import, the viewer. |
| [`PERFORMANCE.md`](PERFORMANCE.md) | Where caditor is slower or heavier than it should be: kernel, STEP import, interface, solver, recompute. |
| [`PLATFORMS.md`](PLATFORMS.md) | Windows checks on a real machine, signing, packaging and distribution. |
| [`CHECKS.md`](CHECKS.md) | Tests, CI and tooling that need attention. |
| [`DECISIONS.md`](DECISIONS.md) | Open decisions: scope directions (assemblies, surfaces, sheet metal) and smaller questions an implementer cannot settle alone. |

## Direction

caditor is a parametric CAD application. The project is judged on two things before
anything else:

1. **User experience.** Modelling should feel direct and predictable, without the failure modes
   that FreeCAD is known for: broken references after an upstream edit, workbench and mode
   juggling, opaque errors and a UI that freezes. The concrete requirements are in
   `.claude/rules/ux.md`.
2. **Never losing work.** Crashes and data loss are catastrophic failures. The concrete
   requirements are in `.claude/rules/reliability.md`.

Features are added only once they meet both bars; an unpolished feature is not shipped. An item
is implemented when it works end to end in the app, not when the APIs exist.

## How the files are kept

- Each file lists only what remains. The change that implements an entry deletes it from its
  file, or narrows it to exactly what is left; a section disappears once it is empty. Git history
  is the record of what was done.
- A resolved decision is recorded in the rules file or `docs/` page covering it and removed from
  `DECISIONS.md`.
- A new finding goes into the file of its kind: a missing capability into `FEATURES.md`, a wrong
  result or a refusal of something that should work into `BUGS.md`, a measured slowness into
  `PERFORMANCE.md`, a question someone else must answer into `DECISIONS.md`. When an entry
  changes kind (a feature found to fail on everyday parts), it moves.
- Within a file, categories run from the most to the least important. Each entry is tagged
  `[importance · ease]`, and within a category the entries that can start now come first, from
  the most to the least important and from the easiest to the hardest. An entry that cannot
  start yet is tagged `blocked by: <blocker>` with the specific item, decision or external fact,
  and follows the unblocked ones; the entry that does the unblocking comes before it. An entry
  tagged `later` instead of an importance is deliberately not a priority: it waits until the rest
  is done, and its category comes last. An entry tagged `next` is taken before every other item,
  whatever its tag, and carries a note saying why; it loses the tag when its change lands, like any
  implemented item.
