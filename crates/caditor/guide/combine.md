# Combine

{command:model.combine} joins, cuts or intersects bodies. Choose Combine, click the body to
combine into (the target), then click the body to combine with it (the tool); the bodies can also
be chosen from the lists in the card above the feature tree. Esc stops choosing. With one body
selected, Combine takes it as the target and asks only for the tool.

When two bodies are already selected (a face or edge of each, Shift or Ctrl adding the second) or
chosen in the tree, Combine joins them straight away: the body picked first in the view is the
target; with bodies chosen in the tree, the one earlier in the tree is.

- **Operation**: Join adds the tool to the target, Cut takes it away, Intersect keeps what they
  share.
- **Target** and **Tool** can be changed in the panel, and **Swap target and tool** exchanges
  them; **Also with** adds more tool bodies.
- **Keep tool** leaves the tool bodies standing instead of using them up.

Cuts made while modelling are often simpler with an extrusion set to Remove from body; see
[Extrude](extrude).
