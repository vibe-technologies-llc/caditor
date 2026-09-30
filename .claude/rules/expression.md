---
paths:
  - "crates/caditor-expression/**"
---

# Units and expressions

- `caditor-expression` has no workspace dependencies; the sketch, document, file and app crates use
  it. An `Expression` refers to parameters by `ParameterId`, never by name, so renaming a parameter
  rewrites every expression's text.

## Quantities and units

- A `Quantity` is an f64 in base units (millimetres and degrees) with a `Dimension` of length and
  angle powers.
- A plain number takes the dimension of whatever it is added to; a field that expects a length takes
  a plain result as millimetres.
- Trigonometry reads a plain number as radians. An angle field takes a plain literal as degrees but
  refuses a computed plain result (`PlainAngle`), asking for deg or rad.
- A unit binds to the primary before it (a number, a parenthesised group, a name or a call,
  `Expression::WithUnit`); `mm²` and `mm³` name areas and volumes.
- Typed text accepts SI units only (`in` and `ft` stay readable in stored text); a misspelled unit
  gets a message naming the units that exist. Names that read as units (`mm²`) are refused as
  parameter names.

## Functions

- Arithmetic and trigonometry, comparisons (plain 1 or 0, equality within 1e-9), lazy `if`, `and`
  and `or`, `not`, `mod`, `hypot`, `exp`, `ln`, `log10`, `log2`, `cbrt`, `sign`, `clamp`, and the
  constants `pi`, `tau` and `e`.
- `round`, `floor`, `ceil` and `trunc` take an optional step; the quotient is snapped to a whole
  number within the comparison tolerance, so `floor(0.3, 0.1)` is 0.3.
- Chained comparisons, decimal commas, `mm2` and `width²` get messages of their own.

## Limits and robustness

- Every intermediate value must be finite and real; a literal too large for an f64 is refused as
  parsed, since it could not be stored.
- Parsing limits length, nesting and the depth of the tree it builds (checked as each operator is
  added, so long chains stop at the limit), so hostile input cannot overflow the stack. Errors are
  plain-language clauses.
- Evaluation walks the tree with an explicit stack of pending nodes and gathered operands, so even
  the deepest stored expression evaluates on a worker's default 2 MiB stack in a debug build.
  Printing, cloning, comparing and dropping recurse but stay well within it; a test holds this.
