# Rendered view

{command:view.style_rendered} draws bodies as they would look in light: a preview of materials,
not a change to the model. Choose it from View › Display style like the other
[display styles](display). Edges are not drawn, but stay pickable as in Shaded without edges.

## What each body shows

- Its colour, the colour of each face that has its own, and its opacity, as the
  [colour and material card](bodies) sets them.
- Its finish, which decides how rough and how metallic the surface is: Polished metal is a sharp
  mirror, Brushed metal a blurred one, Glossy a clear plastic sheen, Matte and Rubber barely shine.
- A body without a finish takes the finish of its material when the material is one of the
  presets (Steel and Aluminium brushed metal, Brass and Copper polished metal, PLA and ABS satin
  and so on), else Satin.

A hovered or selected face is drawn in its highlight colour, lit like the rest, so the selection
always reads (in high contrast with the ordinary shading, keeping its tested colours). Bodies of a failed or outdated feature keep their warning tint. While a
sketch is edited, and while a face analysis or reflection is shown, the view uses the ordinary
shading.

## Lighting

{command:view.lighting} opens the Lighting panel, whose settings are kept in your preferences
(also under Preferences › Appearance › Rendered view):

- **Light follows** the model, as at first, so the sun stays put above it with Z up and shadows
  stay where they fall as you orbit; or the view, so the sun turns round the model with you and
  the side you look at stays lit from any height, top views included.
- **Turned** and **Height** place the sun: turned from behind you (or from X toward Y when it
  follows the model), and how high above the horizon, which is always the model's (Z up).
- **Sun strength** and **Colour temperature**: warm light is around 2700 K, daylight around
  5600 K, an overcast sky 7500 K and above.
- **Sky light** is the light reaching the bodies from the blue sky above and the sunlit ground
  below (or from the sky picture, below, when there is one), which fills shadows and gives
  metals something to reflect. At 0 only the sun lights the model.
- **Bounced light** is sunlight thrown back by nearby lit surfaces, so a red part next to a white
  one tints it. A wall the sun lights stops the light bounced through it, but it reaches only a
  short way and comes only from surfaces the sun sees, so treat it as a hint of colour, not an
  exact result.
- **Ambient occlusion** darkens the sky and bounced light where surfaces crowd in: inside
  corners, holes, the gap under a part. It is worked out from what the view shows (ground-truth
  ambient occlusion), so it only sees surfaces on screen; 0 turns it off, above 1 deepens it.
- **Exposure** brightens or darkens the whole picture in stops.
- **Cast shadows**: bodies shade what lies behind them from the sun.

{command:view.reset_lighting} puts every setting back as it was at first, the sky picture
included.

## Sky and ground

Behind the model the Rendered view shows a blue sky, deeper at the top of the view and paler
below, and leaves out the grid, sketch curves, datums and the principal planes, axes and origin,
so only the bodies show. They come back while a sketch is edited, a feature is open or a plane is
being chosen. The bodies stand on an unseen ground at their lowest point, which shows only the
shadow the sun casts on it, fading away from the model.

Bright light rolls off smoothly rather than clipping to white, keeping a material's colour
recognisable. [Exporting](exporting) an image while the Rendered style is shown draws it with the
same lighting.

## Sky picture

A picture of real or imagined surroundings can replace the blue sky. {command:view.choose_backdrop}
(also Choose picture… in the Lighting panel) opens a Radiance HDR picture, a `.hdr` file whose
width is twice its height, the form most free environment pictures come in. caditor reads it in the
background, so the window stays responsive, then draws it behind the model and lights the bodies
with it: the Sky light setting scales how much, bodies pick up its colours and a glossy or
metallic finish reflects it, sharply for a polished surface and blurred for a rough one. The sun
still lights the model as well, so turn Sun strength down to 0 to let the picture do all of it.

- Z is up, and the middle of the picture lies along the X axis. **Picture turned** turns it
  about Z, in degrees, to bring a window or a lamp to the side you want.
- **Picture brightness** makes it brighter or darker in stops, and the Exposure setting still
  applies to the whole picture.
- The picture is kept in your preferences, so it is there the next time caditor starts. A very
  large picture is shrunk to at most 4096 pixels across for drawing, and one bigger than 8192 by
  4096 pixels is refused: shrink it first.
- If the file cannot be read, is not a Radiance picture or is not twice as wide as it is high,
  the Lighting panel says which file and why, a notice says so too, and the blue sky is drawn
  instead. Other kinds of picture, OpenEXR included, are not read yet.

{command:view.clear_backdrop} (Use the plain sky) goes back to the blue sky, and
{command:view.reset_lighting} does that as well as putting every other setting back.
