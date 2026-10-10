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
- In typed text a power right after a unit raises the unit, as engineers write it: `10 mm^2` is
  `10 mm²`, while `(10 mm)^2` raises the measure. Only a length unit takes a power, and only a
  literal 2 or 3; anything else (`10 deg^2`, `10 mm^4`, `10 mm²^2`) is `UnitPower`, which spells
  the bracketed form.
- Stored text (`parse_stored`) keeps the reading every shipped version wrote it with: a power
  after a unit raises the measure, so an old file's `10 mm^2` is still (10 mm)². Printing brackets
  a power of a measure (`(10 mm)^2`, `ends_in_unit`), which reads alike in both grammars, so new
  files read the same in older versions and no format change was needed. Never print a power
  right after a unit.
- `Expression::parse_by_name_stored_reading` resolves parameter names like typed text but keeps
  the stored grammar (power of the measure, `in` and `ft`); parameter CSV files without a version
  row use it (`file-import-export.md`).
- Typed text accepts SI units only; `in` and `ft` stay readable in stored text (`parse_stored`).
- `Naming::split` reads `name = expression` typed in a value field: a single `=` (not part of
  `==`, `<=`, `>=` or `!=`) after nothing but a name's characters. Anything else is an ordinary
  expression, so a slip like `if(a = b, …)` still gets the parser's message.

## Evaluation

- Comparisons give plain 1 or 0, equal within `EQUALITY_TOLERANCE`; `if`, `and`, `or` are lazy.
- `round`, `floor`, `ceil` and `trunc` take an optional step; the quotient, or the value itself
  without a step, is snapped to a whole number within the tolerance, so `floor(0.3, 0.1)` is 0.3
  and `floor(2.8 mm / 0.4 mm)` is 7. `mod` snaps its quotient the same way, so `mod(0.3, 0.1)` is 0.
- Error messages name the problem in the user's terms: a function given mixed kinds names itself
  (only comparison operators speak of comparing); a length field given `1 / 2 mm` is told that a
  unit binds to the number before it and `(1 / 2) mm` gives the quotient its unit.
- `format_number` output must be readable back by the parser.
- Every intermediate value must be finite; a literal too large for an f64 is refused at parse time.

## Scaling

- `with_lengths_scaled` multiplies each literal by the factor to its length power (`mm²` by the
  square, angles never), and a plain literal by the power it is read at: the field's when the
  whole expression is plain, or the power of what it is added to, compared with or paired with
  in a same-kind function; a plain product or quotient carries it on its plain operand, and a
  plain value that cannot carry it (a parameter, a function) is multiplied by the factor. The
  caller checks the result and falls back when it does not agree (`document.md`).

## Limits

- Parsing bounds length (`MAX_LENGTH`, more for stored text), nesting and tree depth (checked as
  each operator is added), so hostile input cannot overflow the stack.
- Evaluation uses an explicit stack, so the deepest stored expression evaluates on a worker's
  default 2 MiB stack in a debug build. Printing, cloning, comparing and dropping recurse but stay
  within it; a test holds this.
