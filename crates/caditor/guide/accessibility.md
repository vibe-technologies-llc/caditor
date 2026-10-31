# Accessibility

- **Keyboard**: every action is a command reachable from {command:palette}, and the view has
  keyboard views, camera moves, highlighting and typed coordinates; see
  [working from the keyboard](keyboard).
- **Interface size**: {command:view.interface_larger} and {command:view.interface_smaller} scale
  the whole interface from 75% to 200%; {command:view.interface_normal} goes back to 100%. Panels
  and toolbars wrap rather than cut off. Each ribbon group's menu of commands is a button named
  "<group> commands", reached with Tab, that opens with Enter or Space.
- **High contrast** in [Preferences](preferences) › Appearance › Contrast strengthens text,
  outlines and focus rings, and reaches the 3D view. It follows the system by default: while
  Windows uses a contrast theme, or GNOME or KDE ask for more contrast, caditor does too, and it
  can be set on or off for good. It reaches the 3D view: lines and points stand out against the background and bodies,
  a body too dark for its edges and highlighted faces to show is drawn lighter,
  and state is shown by form as well as colour (selected geometry is wider, failed bodies have
  dashed edges, free sketch points are hollow).
- **Reduced motion**: the camera, fades, the interface's own animations and spinners stop moving
  while the desktop turns animations off, or when [Preferences](preferences) › Navigation › View
  changes is No motion.
- **Moving the panel**: {command:view.panel_wider}, {command:view.panel_narrower} and
  {command:view.panel_normal} change the width of the model panel without dragging its edge.
- **Screen readers**: buttons, fields and rows are named, notices and the reason an action was
  refused are announced, and the 3D view is described in words, its bodies, sketches and datums
  each an item to step through; activating an item selects it. The rollback bar is a slider: step
  it to roll the model back or forward. Search fields are named by their placeholder. The view's
  prompt is read with the keys it shows; its **More keys** button, reached with Tab, expands the
  rest and the prompt is read again with all of them.
- **Colour**: nothing is told by colour alone; legends and pills name each state in words.
- **Themes**: every theme, caditor's own and any you add, is checked to keep text readable
  against its background before it is offered. Any of six 3D views, from dark through grey to
  white, can be chosen with any theme, each keeping sketches, edges and highlights easy to see, and
  a grid colour too close to the background to be seen is refused. See
  [Preferences](preferences).
