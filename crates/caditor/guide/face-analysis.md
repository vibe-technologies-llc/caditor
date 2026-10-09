# Face analysis

The face analyses colour the faces of the shown bodies to show what the eye cannot judge. They
are in the View menu and {command:palette}, and open a panel on the right; the switch at its top
moves between them. They never change the model.

- {command:view.analysis_draft} colours faces by their angle to the pull direction, for moulding:
  enough draft, too little, or an undercut. Set the limit angle and the pull from an axis, edge or
  face, or X, Y and Z.
- {command:view.analysis_radius} marks inside curves tighter than a smallest radius, where a cutter
  or nozzle of that radius cannot reach.
- {command:view.analysis_reach} shows what a three-axis machine can reach from one direction:
  reached, blocked by other material, or facing away.
- {command:view.analysis_curvature} colours by Gaussian curvature or by the largest or smallest
  bending, against a reference radius.
- {command:view.analysis_zebra} and {command:view.analysis_chrome} show reflections. Stripes or
  reflections that jump at a joint show a corner, ones that kink show a change in curvature, and
  smooth ones a curvature-continuous joint.

The legend names each colour with the area it covers, so nothing is told by colour alone. Some
display styles do not colour faces; the panel says so.

See also the [curvature comb](curvature-comb) and [isocurves](isocurves).
