# Curvature comb

{command:view.curvature_comb} draws a comb over the selected edges and sketch curves: teeth
standing out from the curve, as long as it bends sharply there. A smooth curve has a smooth comb;
a jump or a kink in the comb shows where the bending changes abruptly.

The comb follows the selection; selecting something else that is not a curve keeps the curves
combed, so you can select a point and drag it to watch the comb change.

- **Teeth per curve** sets how many teeth; **Scale** how long they are.
- The panel lists each curve's smallest radius, and where two combed curves meet, the joint's grade:
  G0 (a corner), G1 (tangent) or G2 (bending alike).

See also [isocurves](isocurves) and [face analysis](face-analysis).
