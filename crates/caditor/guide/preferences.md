# Preferences

{command:file.preferences} opens Preferences. Changes apply at once and are saved for the next
start. **Restore defaults** resets only the tab you are on, and offers Undo. A section you have
changed shows a reset button at the right of its title, which puts back that section's defaults
alone and also offers Undo.

Type in **Search settings**, under the tabs, to see the sections of every tab whose settings
match, for example `zoom`, `grid` or `view cube`. Choosing a tab clears the search.

## General

- **Units**: lengths in micrometres, millimetres, centimetres or metres, and angles in degrees or
  radians. They decide how values are shown and what a plain number you type means; values already
  in a model keep the units they were typed in. See [expressions](expressions).
- **Numbers**: how many decimal places lengths and angles are shown with, and whether trailing
  zeros are shown. **Automatic**, the default, lets each readout show what it needs: measurements
  to a micrometre, the position by the pointer to a hundredth of a millimetre, computed values to
  up to six places. A fixed number of places applies to every readout: Measure, the status bar,
  dimensions and labels in the view, values shown beside fields, the parameter table and hover
  help. Hiding trailing zeros shows 12.500 mm as 12.5 mm. Rounding is only for reading: the model
  keeps every value as you entered it, and a field shows what you typed.
- **Starting and recent files**: whether caditor starts with a new model or opens the last model
  you had, and how many models Open recent lists (3, 5 or 10). Unsaved work offered for
  recovery, or a file named on the [command line](command-line), comes first. **Show the start
  card on empty models** brings back the [start card](start) after Don't show this again, and
  **Show Home when caditor starts** decides whether [Home](cloud) opens first.
- **New models**: the [template](templates) a new model starts from.
- **Feature tree**: what double-clicking a feature does, edit it (as Enter does) or rename it, and
  **New bodies from separate pieces**: whether a new extrusion, revolve or sweep of shapes that do
  not touch makes **One body** of them all (as before) or **A body each**; each feature can still
  be switched in its panel's Separate pieces row.
- **Version history**: how many earlier versions a save keeps inside the file, more, standard or
  fewer; see [version history](version-history). Versions you keep always stay, and unsaved work
  is always kept for recovery.
- **Exports**: whether the export dialogs keep their format, options and folder between sessions
  (the default) or only until caditor closes, and **Forget remembered choices**, after which the
  next session's dialogs start from caditor's defaults, and **Slicer program**, the OrcaSlicer
  program or AppImage that Open in OrcaSlicer starts when caditor does not find it by itself; while
  the field is empty it says where caditor found OrcaSlicer, or that it did not. See
  [exporting](exporting).
- **Cloud**: whether [cloud services](cloud) are used at all (turn them off here to hide every
  cloud button, sign out and make no cloud request), whether projects show on Home, and the
  Autodesk app to sign in with when the build carries none. Restore defaults never turns cloud
  services back on.
- **Keyboard**: the shortcut editor; see [working from the keyboard](keyboard).
- **Tips**: show tips in the view, and bring back dismissed ones.
- **Saving on Windows**, on Windows only: shows again the reminder about
  [Microsoft Defender](windows-defender).

## Appearance

- **Theme**: a card for each theme, each a small picture of how the window and the 3D view will
  look: follow the system, Dark, Light, Midnight, Graphite, Paper, Slate (dark panels with a
  mid-grey 3D view), Studio (light panels with a light grey 3D view), and any themes of your own.
  Click a card, or move between them with the arrow keys and press Enter; the whole window
  changes at once.
- **Accent**: the colour of each panel's or dialog's main button, of selections, links and the
  keyboard focus, the theme's own or one of six others.
  An accent that would make text hard to read in the chosen theme is greyed out.
- **3D view**: the background of the 3D view, a swatch for each: with the theme (Paper and Studio
  have light ones, Slate a grey one), or always Dark, Grey, Studio grey, Blue-grey, Light or White,
  whatever the theme. The darker two are drawn in light lines, the others in dark lines, each
  checked to keep sketches, edges and highlights easy to see.
- **Grid**: the colour of the grid in the 3D view. **Grid: automatic** uses the 3D view's own grid
  colour; the plus swatch opens a colour picker for a colour of your own, which applies at once. A
  colour too close to the background to be seen (below 1.5:1, or 3:1 with high contrast) is
  refused, saying why. If you later choose a 3D view your colour does not show on, the view's own
  grid is drawn and Preferences says so until you choose another colour or Automatic.
- **Bodies**: the colour of bodies that have no colour of their own, new ones included: caditor's
  grey, one of the body swatches, or your own from the plus swatch. A colour too close to the 3D
  view to stand out (below 1.5:1) is refused, saying why; if you later choose a 3D view it does not
  show on, bodies are drawn grey and Preferences says so. It changes only how caditor shows
  them: the model file and exports keep only colours you give a body in its
  [colour and material](bodies) card.
- **High contrast**: stronger text, outlines and focus, in the panels and in the 3D view. It uses
  its own colours in place of the theme's.
- The interface size from 75% to 200%, whether caditor draws its own title bar or uses the
  system's, and how soon hover help appears: at once, after about a third of a second, or after a
  second. See [accessibility](accessibility).
- **In the 3D view**: thin, standard or thick body edges, whether the outlines of round faces are
  drawn, whether the view cube with Fit all and the Isometric view button is shown, how large
  it is (small, standard or large) and which corner of the view it sits in (in a bottom corner the
  axes or the mouse hints move aside to make room for it), and **Show principal planes only when
  needed**, on at first: see [display styles and hiding](display).

## Your own themes

A theme is a JSON file in the `themes` folder of caditor's configuration folder (the folder is
named under the theme cards). Click **Reload themes** after adding or changing one. A file holding
`{ "name": "Ocean", "base": "dark", "view": "studio", "grid": "#5a6070", "colours": { "panel": "#0b2230" } }`
adds a theme called Ocean.

`base` is `dark` or `light`, and its colours fill in whatever the file leaves out; `view` is the
3D view: `dark`, `grey`, `studio`, `blue_grey`, `light` or `white`. `grid`, which may be left out,
is the grid's colour written `#rrggbb`; it must show on that view as a grid colour you choose
must. The colours are named `panel`, `raised`, `sunken`, `stripe`, `button`,
`hover`, `pressed`, `border`, `border_strong`, `field_border`, `text`, `text_muted`,
`text_on_accent`, `accent`, `accent_hover`, `accent_pressed`, `accent_text`, `accent_subtle`,
`accent_surface`, `focus`, `error`, `error_subtle`, `danger`, `danger_hover`, `danger_pressed`,
`warn`, `warn_subtle`, `success`, `success_subtle` and `shadow`, each written `#rrggbb`.

Every theme is checked for readable contrast, as caditor's own are. A theme that fails, or cannot
be read, is not offered: Preferences says which file and why, naming the two colours that are too
close, for example the muted text on a button, or a grid the 3D view would hide. If the theme you chose stops working, caditor keeps
the one you had and says why.

## Navigation

- **View**: the projection; the field of view, narrow (20°), standard (30°) or wide (45°), for
  less or more perspective; and how view changes move: following the system, standard, quick, slow
  or with no motion at all, so standard views, fitting and opening a sketch jump straight to the
  new view. Following the system glides at the standard speed, and jumps instead while the
  desktop asks for less motion (the reduced motion setting of desktops that offer one, GNOME's
  Enable Animations, KDE Plasma's animation speed set to instant, Windows' Animation effects),
  and notices a change in the desktop's settings straight away; a desktop that does not say keeps
  the glide.
- **Input**: the mouse input mode: caditor, Laptop, Fusion 360, FreeCAD, Blender or SolidWorks.
- **Movement**: orbit and zoom speeds, the zoom direction, whether zooming goes toward the pointer
  or keeps the view's centre still, whether orbiting turns about the point under the pointer or
  the middle of the view, and turning either orbit direction round.

See [moving around the view](navigation).

## Sketching

- **Editing a sketch**: whether opening a sketch turns the view to face it, and whether Escape
  finishes the sketch once there is nothing left to cancel. Off, Escape still cancels shapes,
  tools and the selection, and Finish sketch finishes.
- **Constraints drawing infers**: which kinds a point placed while drawing may be held to, each
  with its own box: horizontal, vertical, parallel, perpendicular and tangent lines; coincident
  with a point, on a curve, at a midpoint, at the centre of an outline, at an intersection and at a
  quadrant or axis end; and in line with a point the pointer passed over. All start on. An unticked
  kind is neither offered while drawing nor made a constraint; the constraint tools still add it by
  hand. See [snapping](snapping).
- **On when caditor starts**: constraint glyphs, snapping, snapping to the grid, keeping typed
  values as dimensions, and dimensions shown on the model. Each can still be turned on or off
  while you work; changing one here changes it now too. See [snapping](snapping) and
  [dimensions](dimensions).

## Graphics

Vsync, a frame-rate limit, anti-aliasing, shading and how smooth curved faces are drawn, and which
graphics adapter to prefer. An option the adapter cannot do is shown disabled with the reason. The
tab ends with the adapter's details and a button to copy them for a bug report.

Frame rate is No limit (the default), Fixed rate, which shows a slider for any rate from 1 to 1000
frames a second, or Match the display. A fixed rate also caps drawing while Wait for the display is
on, which saves power on a fast display; Match the display only limits drawing when Wait for the
display is off.

The adapter preference is Auto, Power saving or Performance. Auto, the default, picks the most
powerful graphics card on a computer without a battery and the integrated graphics on a laptop,
and a line under it says what it chose and why, for example "Auto: high performance, this computer
has no battery." When caditor cannot tell, Auto takes the integrated graphics. Power saving and
Performance always do what they say.
Changing the preference switches the drawing to the new adapter at once, only if it is a different
one. Setting the `WGPU_POWER_PREF` environment variable to `low` or `high` overrides the
preference.
