---
paths:
  - "crates/caditor-expression/**"
---

# Units and expressions

- No workspace dependencies. An `Expression` refers to parameters by `ParameterId`, never by name,
  so renaming a parameter changes the rendered text, not the tree.

## Quantities and units

- A `Quantity` is an f64 in base units (millimetres, degrees) with a `Dimension` of length and
  angle powers.
- A plain number takes the dimension of whatever it is added to; a length field takes a plain
  result as millimetres. Trigonometry reads a plain number as radians. An angle field takes a
  plain literal as degrees but refuses a computed plain result (`PlainAngle`), asking for deg or
  rad.
- A literal is never negative: the sign is an `Expression::Negate` around it, as the parser builds
  it, since stored text spells both alike. Build literals with `Expression::number` and
  `Expression::measure`, so an expression reads back from a file or the journal exactly as it was.
- A unit binds to the primary before it (`Expression::WithUnit`); `mm²` and `mm³` name areas and
  volumes. Names that read as units are refused as parameter names (`check_name`).
- Typed text accepts SI units only; `in` and `ft` stay readable in stored text (`parse_stored`).

## Evaluation

- Comparisons give plain 1 or 0, equal within `EQUALITY_TOLERANCE`; `if`, `and`, `or` are lazy.
- `round`, `floor`, `ceil` and `trunc` take an optional step; the quotient is snapped to a whole
  number within the tolerance, so `floor(0.3, 0.1)` is 0.3.
- Error messages name the problem in the user's terms: a function given mixed kinds names itself
  (only comparison operators speak of comparing); a length field given `1 / 2 mm` is told that a
  unit binds to the number before it and `(1 / 2) mm` gives the quotient its unit.
- `format_number` output must be readable back by the parser.
- Every intermediate value must be finite; a literal too large for an f64 is refused at parse time.

## Limits

- Parsing bounds length (`MAX_LENGTH`, more for stored text), nesting and tree depth (checked as
  each operator is added), so hostile input cannot overflow the stack.
- Evaluation uses an explicit stack, so the deepest stored expression evaluates on a worker's
  default 2 MiB stack in a debug build. Printing, cloning, comparing and dropping recurse but stay
  within it; a test holds this.
