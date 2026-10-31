# caditor accessibility

What caditor does for people who use assistive technology, a keyboard alone, a large interface, high
contrast or less motion; which standards each part answers; and how it is checked. It is a
conformance statement of the project's own, not a third-party audit or a certification.

Standards used as the yardstick: WCAG 2.2 level AA read for software as WCAG2ICT does, EN 301 549
(chapters 5, 11 and 12 apply to a downloaded desktop application) and Section 508 (which adopts the
same WCAG and EN 301 549 requirements for software, 502 and 503). Where a criterion is about web
pages only it is left out. The design rules are in `.claude/rules/ux.md`; this page maps them to
the criteria and to the check that keeps each one true.

Each row says **Supported** (and where the check lives), **Partly** (what is missing), or **Not
verified** (needs a person with the assistive technology; see the last section).

## How it works

- The window exposes an accessibility tree through AccessKit (`egui-winit`'s `accesskit`
  feature): UIA on Windows, AT-SPI on Linux. It is created on request, when an assistive
  technology connects (`overlay.rs`), so it costs nothing otherwise.
- Every control is named without its icon glyph; decorative glyphs are hidden; notices, the failed
  and waiting feature pills, the prompt and a refusal note are live regions.
- The 3D view is a named region with a description, and a list of its bodies (each with its
  faces), sketches, datums and, while a sketch is edited, its curves and points. Activating an item
  selects what it describes.
- Every action is a command in the palette; the view has keyboard views, camera moves, highlighting
  and typed coordinates.

## Settings that follow the system

| Setting | Follows | Where |
| --- | --- | --- |
| High contrast | Windows contrast themes; the freedesktop `contrast` setting (GNOME, KDE) | Preferences › Appearance › Colours › Contrast (Follow the system, Standard, High contrast) |
| Reduced motion | Windows animation effects; freedesktop `reduced-motion`, GNOME `enable-animations`, KDE `AnimationDurationFactor` | Preferences › Navigation › View changes (Follow the system, No motion, Quick, Standard, Slow) |
| Interface size | Chosen in the application, 75% to 200% | Preferences › Appearance › Interface, View › Window and interface |

Reduced motion covers camera moves, the fade of principal geometry, the interface's own
animations (egui's animation time is set from the same value) and the spinner, which stops turning
and keeps its name and role.

## WCAG 2.2 and WCAG2ICT

| Criterion | Status |
| --- | --- |
| 1.1.1 Non-text content | Supported: icon buttons are named by their hover text, decorative glyphs are hidden, pictures are named (`ui_tests::conformance::every_control_on_every_screen_has_a_name_a_screen_reader_can_speak`, `assert_readable`). |
| 1.2 Time-based media | Not applicable: no audio or video. |
| 1.3.1 Info and relationships | Supported: captions label their fields, tabs and lists have roles. Partly: feature tree rows are selectable buttons, not tree items (see Known limits). |
| 1.3.2 Meaningful sequence | Supported: the Tab walk visits every screen in reading order and comes round (`tab_comes_round_on_every_screen_without_a_trap_or_focus_off_screen`). |
| 1.3.3 Sensory characteristics | Supported: instructions name keys and actions, never only a shape or colour. |
| 1.4.1 Use of colour | Supported: every status tone has its own icon and words; selection widens, failed bodies are dashed, free points are hollow. `colour_vision.rs` simulates protan, deutan and tritan vision and fails when two tones that look alike share an icon. |
| 1.4.3 Contrast (minimum) | Supported: `appearance::check` holds every text pair of every theme, shipped or user, to 4.5:1. |
| 1.4.4 Resize text | Supported: the interface scales to 200% without loss (`large_interface`, `command_sweep`). |
| 1.4.5 Images of text | Supported: text is text. |
| 1.4.11 Non-text contrast | Supported: outlines, focus rings and 3D view lines hold 3:1 (`appearance::check`, `scene_palette` tests). |
| 1.4.13 Content on hover or focus | Partly: hover help is persistent while the pointer is on it and its delay is a preference; dismissing it with Escape is not verified. |
| 2.1.1 Keyboard | Supported: every command is in the palette (`keyboard_driving`, `command_sweep`). The 3D view is driven by highlighting, activating and typed values. |
| 2.1.2 No keyboard trap | Supported: the Tab walk comes round on every screen; Escape leaves dialogs. |
| 2.1.4 Character key shortcuts | Supported: single keys act only when no text field has focus, and every binding can be changed or removed in the shortcut editor. |
| 2.2.1 Timing adjustable | Supported: notices stay until the next edit or until dismissed; the one timed message, a refusal note beside the pointer, can be set to stay until a key or click (Preferences › Appearance › Interface › Refusal notes) and is also posted as a notice. |
| 2.2.2 Pause, stop, hide | Supported: nothing moves on its own except progress indicators, which stop under reduced motion. |
| 2.3.1 Three flashes | Supported: nothing flashes; the text caret does not blink. |
| 2.4.3 Focus order | Supported: see 1.3.2. |
| 2.4.6 Headings and labels | Supported: sections, dialogs and fields carry plain titles and captions. |
| 2.4.7 Focus visible | Supported: one focus ring in the `focus` token, held to 3:1 and strengthened in high contrast. |
| 2.4.11 Focus not obscured | Supported in side panels, menus and dialog bodies: keyboard or assistive technology focus scrolls into view (`widgets::follow_keyboard_focus`, checked by the Tab walk). Partly: other scrolling lists (version history, saved views and similar) rely on egui. |
| 2.5.1 Pointer gestures | Supported: orbit, pan and zoom have key commands; box, lasso and paint selection have highlighting and Activate. |
| 2.5.3 Label in name | Supported: a control is named by its visible label, and an icon-only control by the text it shows on hover. |
| 2.5.7 Dragging movements | Supported: the rollback bar is a slider with increment and decrement and has commands; tree order has commands; panel width has commands (Make the model panel wider, narrower, usual width); handles take typed values. |
| 2.5.8 Target size (minimum) | Supported at 100% and 200%, tested on the model, a sketch, the palette, the preferences and the window of every command (`every_target_is_at_least_24_points_or_clear_of_the_others…`, `every_window_of_the_…_quarter_names_its_controls_and_sizes_its_targets`). Partly: the ribbon group titles (14 points tall) and the corner marks that open a tool's ways are smaller, and are allowed only because every function they open is also a command in the menu bar and the palette; the test's `EQUIVALENT_ELSEWHERE` table lists each and its route. At 75% interface size targets are 75% as large. |
| 3.2 Predictable | Not verified: commands run only when asked, but focus behaviour after every change has not been surveyed. |
| 3.3.1 to 3.3.3 Errors | Supported: errors say what happened, to which feature or entity and what to do next (`ux.md`). |
| 3.3.4 Error prevention | Supported: unsaved changes are asked about, every change is undoable, saves keep earlier versions. |
| 4.1.2 Name, role, value | Supported for buttons, fields, check boxes, tabs, links, sliders (the rollback bar, the hue strip), lists and the view. Partly: see Known limits. |
| 4.1.3 Status messages | Supported: notices, progress, the prompt and refusal notes are live regions (polite, assertive only for errors). |

## EN 301 549 and Section 508

- 5.2 Activation of accessibility features and 502.2.2 No disruption: nothing in caditor turns a
  system accessibility feature off; the accessibility tree starts when a technology asks.
- 5.5 Operable parts, 5.6 Locking or toggle controls: toggles show their state in menus
  (checkmarks) and in the palette (On or Off).
- 5.7 Key repeat: only camera commands repeat; the system's repeat settings apply.
- 5.9 Simultaneous user actions: not verified. Alt-drag and Shift-click are modifier-plus-pointer
  actions; the commands and typed values behind them are one-handed.
- 11.x: the WCAG rows above. 11.7 User preferences: contrast and motion follow the system; the
  interface size is the user's own.
- 12.1 Product documentation: the user guide is inside the application and uses the same widgets,
  so it is as accessible as the rest; `guide/accessibility.md` describes the features.
- 502.3 and 503 Interoperability and applications: AccessKit's tree for assistive technology.
- Not applicable: closed functionality (5.1), biometrics (5.3), two-way voice (6), video (7) and
  hardware (8).

## Known limits

- Feature tree rows, the Bodies rows and parameter rows are selectable buttons, not tree items or
  table rows; a screen reader reads them but not their depth or expansion as a tree.
- The 3D view's list holds at most `ITEMS_AT_MOST` items and a body's first `FACES_AT_MOST` faces;
  the view's description says how many more there are and to step through them with Highlight
  next.
- The window's own move and resize (with caditor's title bar) are pointer or window manager
  operations; the system title bar can be chosen instead (Preferences › Appearance › Interface ›
  Title bar).
- Text size cannot be changed apart from the whole interface's size; line and letter spacing cannot
  be overridden.
- The lock file lists two `egui` and two `accesskit` versions because the icon font crate trails
  `egui`; `deny.toml` skips the duplicate.

## Checks

Run by `cargo test --workspace`:

- `ui_tests/accessibility.rs`: names, live regions, the 3D view's list and its activation, the
  rollback slider, contrast following the system, motion following the preference and the system,
  refusal notes, the panel width commands.
- `ui_tests/conformance.rs`: target size, Tab order and visible focus, and names, over the main
  screens and the window every command opens.
- `colour_vision.rs`: status tones that look alike to a colour-blind eye have different icons.
- `appearance.rs` and `scene_palette.rs`: contrast of every theme, canvas and palette.

## Checking with assistive technology

Automated checks read the accessibility tree; they cannot hear it. Before a release, with a person
who uses it daily where possible, on each of Orca (Linux, GNOME and KDE), NVDA and Narrator
(Windows):

1. Start the screen reader first, then caditor; then start caditor first and the screen reader
   second. The window must be named and its controls listed in both orders.
2. Walk the menu bar, ribbon, feature tree and a dialog with Tab and arrows; each stop is spoken
   with a name and a role, and focus is visible.
3. Open a sample, move to the 3D view's list, step through the bodies, activate one and hear the
   selection in the status bar.
4. Cause a refusal (a rectangle with no size) and a failed feature; both are spoken once.
5. Turn on the system's high contrast and reduced motion settings while caditor runs; both take
   effect without a restart (Windows contrast themes, `org.freedesktop.appearance` on Linux).
6. Set the interface to 200% and repeat step 2.

Record the technology, its version, the desktop and what differed in the issue that tracks it.
Windows checks need a real Windows machine (`docs/PLATFORMS.md`).
