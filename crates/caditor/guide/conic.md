# Conic

{command:sketch.conic} draws an exact conic curve, the shape of a section through a cone: an
ellipse arc, a parabola or a hyperbola arc. Click its start, its end, then the apex, where the
tangents at both ends meet.

Its fullness is **rho**, between 0.01 and 0.99: below 0.5 the curve is an ellipse arc, at 0.5 a
parabola, above it a hyperbola arc that hugs the apex more closely. Type it in the point field as
`0.3 rho` while drawing; the preview follows as you type, and a value from
[parameters](parameters) keeps the conic driven by them.

Dimensioning a lone conic adds its rho as a dimension you can edit later. Constrain its ends and apex
like any points, and use **Tangent** to make it run on from the curves at its ends; see
[Spline](spline) for its control polygon.
