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
  look: follow the system, Dark, Light, Midnight, Graphite, Paper, and any themes of your own.
  Click a card, or move between them with the arrow keys and press Enter; the whole window
  changes at once.
- **Accent**: the colour of buttons, selections and links, the theme's own or one of six others.
  An accent that would make text hard to read in the chosen theme is greyed out.
- **3D view**: dark or light with the theme (Paper has a light one), or always dark or light.
- **High contrast**: stronger text, outlines and focus, in the panels and in the 3D view. It uses
  its own colours in place of the theme's.
- The interface size from 75% to 200%, and whether caditor draws its own title bar or uses the
  system's. See [accessibility](accessibility).

### Your own themes

A theme is a JSON file in the `themes` folder of caditor's configuration folder (the folder is
named under the theme cards). Click **Reload themes** after adding or changing one.

```json
{
  "name": "Ocean",
  "base": "dark",
  "view": "light",
  "colours": { "panel": "#0b2230", "raised": "#10293a", "accent": "#1f6fb2" }
}
```

`base` is `dark` or `light`, and its colours fill in whatever the file leaves out; `view` is the
3D view's lightness. The colours are named `panel`, `raised`, `sunken`, `stripe`, `button`,
`hover`, `pressed`, `border`, `border_strong`, `field_border`, `text`, `text_muted`,
`text_on_accent`, `accent`, `accent_hover`, `accent_pressed`, `accent_text`, `accent_subtle`,
`accent_surface`, `focus`, `error`, `error_subtle`, `danger`, `danger_hover`, `danger_pressed`,
`warn`, `warn_subtle`, `success`, `success_subtle` and `shadow`, each written `#rrggbb`.

Every theme is checked for readable contrast, as caditor's own are. A theme that fails, or cannot
be read, is not offered: Preferences says which file and why, naming the two colours that are too
close, for example the muted text on a button. If the theme you chose stops working, caditor keeps
the one you had and says why.

## Navigation

The mouse input mode, orbit and zoom speeds, the zoom direction and the projection. See
[moving around the view](navigation).

## Graphics

Vsync, a frame-rate limit, anti-aliasing, shading and how smooth curved faces are drawn, and which
graphics adapter to prefer. An option the adapter cannot do is shown disabled with the reason. The
tab ends with the adapter's details and a button to copy them for a bug report.
