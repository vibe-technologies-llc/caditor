# Preferences

{command:file.preferences} opens Preferences. Changes apply at once and are saved for the next
start. **Restore defaults** resets only the tab you are on, and offers Undo.

## General

- **Units**: lengths in micrometres, millimetres, centimetres or metres, and angles in degrees or
  radians. They decide how values are shown and what a plain number you type means; values already
  in a model keep the units they were typed in. See [expressions](expressions).
- **New models**: the [template](templates) a new model starts from.
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
- **Accent**: the colour of buttons, selections and links, the theme's own or one of six others.
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
- **High contrast**: stronger text, outlines and focus, in the panels and in the 3D view. It uses
  its own colours in place of the theme's.
- The interface size from 75% to 200%, and whether caditor draws its own title bar or uses the
  system's. See [accessibility](accessibility).

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

The mouse input mode, orbit and zoom speeds, the zoom direction and the projection. See
[moving around the view](navigation).

## Graphics

Vsync, a frame-rate limit, anti-aliasing, shading and how smooth curved faces are drawn, and which
graphics adapter to prefer. An option the adapter cannot do is shown disabled with the reason. The
tab ends with the adapter's details and a button to copy them for a bug report.

The adapter preference is Auto, Power saving or Performance. Auto, the default, picks the most
powerful graphics card on a computer without a battery and the integrated graphics on a laptop,
and a line under it says what it chose and why, for example "Auto: high performance, this computer
has no battery." When caditor cannot tell, Auto takes the integrated graphics. Power saving and
Performance always do what they say, and a choice saved by an earlier version keeps applying.
Changing the preference switches the drawing to the new adapter at once, only if it is a different
one. Setting the `WGPU_POWER_PREF` environment variable to `low` or `high` overrides the
preference.
