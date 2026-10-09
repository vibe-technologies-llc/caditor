# Getting started

caditor is a parametric modeller: a model is a list of features, each built from the ones before
it and from values you can change at any time. Change a value and everything after it follows.

## A first part

- Choose {command:model.new_sketch} and click a plane in the view, such as the XY plane.
- Draw a closed outline with the [Line](line), [Rectangle](rectangle) or [Circle](circle) tool.
- Add [dimensions](dimensions) and [constraints](constraints) until nothing is left free; the
  [sketch's status](sketch-status) says how much can still move.
- Choose {command:sketch.finish}, then {command:model.extrude} to turn the outline into a body. Type
  its distance in the panel that opens and press Enter.
- Add [fillets](fillet-and-chamfer), [holes](hole) or more [sketches](sketches) on its faces.

Every value takes [expressions](expressions) and named [parameters](parameters), so `width / 2`
or `plate + 3 mm` work wherever a number does.

## Never losing work

Each change is written to a [recovery journal](saving) beside the file as you work, and saving
keeps the earlier versions inside the file ([version history](version-history)). If caditor or the
computer stops, the next start offers to restore what was not saved. Every change can be
[undone](undo).

## Finding your way

Press {command:palette} to find any command by name. Press {command:help.guide} at any time for the
page about the tool or panel in use. {command:help.welcome} opens finished samples to look at.

Read on with [the window](window), [moving around the view](navigation) and
[selecting](selection).
