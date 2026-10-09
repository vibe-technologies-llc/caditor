# Coordinate systems

{command:model.coordinate_system} adds a set of three axes at an origin. It is in the Model menu
under Datums and in {command:palette}.

It takes from the selection a point for its origin, an axis, edge or sketch line for its X axis and
a plane or flat face for its XY plane; whatever is not selected is chosen to suit what is. The
panel changes each, and **Reverse X** and **Reverse Z** flip it.

Its axes and planes can be picked like datums, so sketches, patterns, mirrors and revolves can use
them. Moves, scales, imports and [Measure](measure) can work along its axes instead of the
world's.
