# The window

## Menus and ribbon

The menu bar holds File, Edit, View, Model, Sketch and Help, with the model's name in the middle
and, at the right, the gear that opens Preferences and Search commands. Related commands sit
together in submenus. Click the model's name for its details: where it is saved,
Save, Version history and Model properties.

The ribbon under it holds the most used tools in groups: History, Sketch, Solid, Modify, Pattern,
Reference and Analyse. Hover a button to see what it does, its shortcut, or why it is not available
now. Click a group's name, with the small arrow beside it, to open a menu of every command the group has,
the ones without a button of their own included, each with its shortcut; choose one to run it as
its button would. Escape or a click elsewhere closes the menu, and with the keyboard the arrow keys
move through it and Enter runs the one highlighted. The Analyse group holds Measure, Check
interference, the face analyses, the curvature comb, isocurves, the section view and the centres
of mass, and its menu marks the ones that are open. While a sketch is edited a second ribbon holds
the sketch tools in History, Select, Draw, Modify, Constrain and Dimension, with its own Undo and
Redo. Its groups open menus the same way: every drawing tool and every way to draw a shape, every
modify tool and sketch edit, every constraint, the dimension tools and constraining
automatically, with the tool in use and the current way of each shape marked. A small arrow or
triangle in a button's corner opens a menu of more tools or ways to draw; hover it to see what the
menu holds. When the window is short for the interface size, the ribbons show their tools as icons
alone, named on hover, so the view keeps its room, and each group's menu opens from a small arrow
or triangle in the corner of one of its tools (its last on the main ribbon; on the sketch ribbon
Redo, Select, Line, Delete, the last constraint and Disable). Tools without a button are in the
menus and in {command:palette}.

## The model panel

On the left, the model panel lists the Bodies, the Features and the Parameters. The
[feature tree](feature-tree) shows every feature in the order it is built; opening one shows its
settings under its row. [Parameters](parameters) are named values the model uses.

## The 3D view

The view shows the model. The view cube in its corner turns it and Fit all under the cube frames
everything; see [moving around the view](navigation) and [display styles](display). Tips appear at
the bottom while you learn; [Preferences](preferences) brings dismissed ones back.

While the graphics card is still taking in bodies, a large model or a body you just changed shows
up late. A small "Loading to the graphics card" note under the view cube and a spinner in the
status bar say that more is coming, and both go as soon as the last body is in.

## Side panels

Measure, Check interference, the face analyses, the curvature comb, isocurves, Constrain
automatically and this guide open as panels on the right. Together they never take more than 60%
of the window, so the view keeps its room. Drag a panel's edge to widen it; the panels on the right
open at that width from then on, also the next time caditor starts.

## The status bar

From the left: the recompute (with Cancel while it runs, or the failed features and, in blue, how
many features are waiting for you to choose something; click either to go to the next one), file
work such as saving or exporting or loading bodies to the graphics card, and the latest message.
Work that can be stopped, such as a recompute, an import, an export or a paste, has a Cancel
beside it. On the right: what is selected, its size or the distance between two selected items, the
[selection filter](selection) while one is on (click it to select anything again), the length unit
(click it for [Preferences](preferences)) and the interface size when it is not 100% (click it to
go back to 100%). Hovering a button there names its keys.

The size depends on what is selected: a circle or round edge gives its diameter, radius and
circumference, an arc its radius, length and sweep, a line its length, a rectangular face or
region its width × height and area, a round face its diameter, any other flat face or region its
perimeter and area, and a body its size along each axis.

A message's icon says what kind it is: information, done, a warning that something was not done
or only partly, or an error. The button beside its close button opens
{command:help.messages}, which lists earlier messages, so a failed save is never missed.
