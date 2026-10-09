# Arcs

Three tools draw arcs. They share one button on the sketch ribbon, which shows the one used last;
each keeps its own key.

- {command:sketch.arc}: click the centre, the start and the end. The arc runs the way the pointer
  sweeps round the centre.
- {command:sketch.three_point_arc}: click the start and the end, then a point the arc passes
  through.
- {command:sketch.tangent_arc}: start on the end of a line, arc or spline; the arc leaves along
  that curve's direction and stays tangent to it. Arcs chain one after another until Enter or Escape.

{command:sketch.reverse_arc} sends the arc being drawn the other way round. The radius and sweep
show beside the pointer.

Switching between Line and Tangent arc in the middle of a chain keeps the chain going, so an
outline of lines and rounded turns is drawn without stopping. See [Line](line).
