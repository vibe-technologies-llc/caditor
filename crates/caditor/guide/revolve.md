# Revolve

{command:model.revolve} turns the closed regions of a sketch about an axis into a body, or cuts
them into one.

The axis is the line, construction line, sketch axis, datum axis, principal axis, straight edge or
round face selected last; select it together with the sketch before choosing Revolve. In the
panel, {command:model.use_selected_axis} or Choose in the view changes it.

## Extent

- **Full turn**, **One side** by an angle, **Symmetric** both ways, or **Two sides** each with its
  own angle. Two angles together may not pass a full turn.
- **Up to face** turns until the profile reaches a face or plane that contains the axis, forward or
  reversed.
- {command:model.reverse_direction} turns a one-sided or Up to face revolve the other way.
- **Profile**: Whole, or One side keeping only the regions on one side of the axis; Keep the other
  side of the axis swaps.
- **Start** begins at the sketch plane or another face or plane, as for [Extrude](extrude).

## Result

**New body**, **Add**, **Remove from body** or **Intersect with body**, as for an extrusion, and a
**Thin wall** fill for a shell of revolution.

A construction line in the sketch makes a good axis: it guides the drawing and never makes a
region.
