# caditor checks

Tests, CI and tooling that are missing or need attention.

Entries are tagged and ordered as `ROADMAP.md` describes.

- [medium · medium] `ui_tests::emboss::clearing_the_chosen_regions_waits_for_regions_and_the_palette_selects_all_again`
  fails in about half of full `cargo test --release -p caditor` runs on a loaded machine and never
  alone: after clicking Clear and settling, the emboss still holds its one chosen region
  (`emboss.rs:385`), so the click is lost or lands before the button is there. Once seen beside it,
  `paste_picks::a_running_paste_shows_one_polite_progress_line_whose_cancel_stops_it` failed too.
- [high · medium · blocked by: the Autodesk app (`DECISIONS.md`)] The Fusion 360 calls follow
  Autodesk's published documentation and are tested against JSON of the shapes it describes
  (`caditor-cloud`'s `fusion/tests.rs`), never against Autodesk's servers with a registered app.
  With one, run once a sign-in, a team hub with several projects, a nested folder, a conversion
  and a download of a large design, and keep any shape that differs as a test.
