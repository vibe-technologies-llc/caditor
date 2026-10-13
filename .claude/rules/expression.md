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
- A plain number takes the dimension of whatever it is added to; a length field reads a plain
  result as millimetres. Trigonometry reads a plain number as radians. An angle field takes a
  plain literal as degrees but refuses a computed plain result (`PlainAngle`), asking for deg or
  rad.
- A literal is never negative: the sign is an `Expression::Negate` around it, as the parser builds
  it. Build literals with `Expression::number` and `Expression::measure` so an expression reads
  back from a file or the journal exactly as it was.
- A unit binds to the primary before it (`Expression::WithUnit`); `mm²` and `mm³` name areas and
  volumes. Names that read as units are refused as parameter names (`check_name`).
- Typed text: a power right after a unit raises the unit (`10 mm^2` is `10 mm²`), while
  `(10 mm)^2` raises the measure. Only a length unit takes a power, and only a literal 2 or 3;
  anything else (`10 deg^2`, `10 mm^4`, `10 mm²^2`) is `UnitPower`, which spells the bracketed form.
- Stored text (`parse_stored`) has the typed grammar, with parameters as `$<id>` instead of names
  and a longer limit (`MAX_STORED_LENGTH`). Printing brackets a power of a measure (`(10 mm)^2`,
  `ends_in_unit`); never print a power right after a unit. Changing the grammar so stored text
  would read differently changes the file formats (`file-format.md`).
- Text accepts SI units only, typed or stored: `in` and `ft` are `ImperialUnit`, refused with the
  SI units to write instead.
- `Naming::split` reads `name = expression` in a value field: a single `=` (not part of `==`,
  `<=`, `>=`, `!=`) after nothing but a name's characters. Anything else is an ordinary
  expression, so `if(a = b, …)` still gets the parser's message.

## Evaluation

- Comparisons give plain 1 or 0, equal within `EQUALITY_TOLERANCE`; `if`, `and`, `or` are lazy.
- `round`, `floor`, `ceil` and `trunc` take an optional step; the quotient (or the value without
  a step) is snapped to a whole number within the tolerance, so `floor(0.3, 0.1)` is 0.3 and
  `floor(2.8 mm / 0.4 mm)` is 7. `mod` snaps its quotient the same way.
- Error messages use the user's terms: a function given mixed kinds names itself (only comparison
  operators speak of comparing); a length field given `1 / 2 mm` is told a unit binds to the
  number before it and `(1 / 2) mm` gives the quotient its unit.
- `format_number` output must be readable back by the parser. A number with a fixed count of
  decimals goes through `format_decimals` (trims trailing zeros, never writes `-0`) or, keeping its
  zeros, `format_places` (never writes `-0.000`); `trim_decimals` trims text already formatted. No
  other crate trims zeros itself.
- Every intermediate value must be finite; a literal too large for an f64 is refused at parse time.

## Scaling

- `with_lengths_scaled` multiplies each literal by the factor to its length power (`mm²` by the
  square, angles never), and a plain literal by the power it is read at: the field's when the
  whole expression is plain, else the power of what it is added to, compared with or paired with
  in a same-kind function; a plain product or quotient carries it on its plain operand, and a
  plain value that cannot carry it (a parameter, a function) is multiplied by the factor. The
  caller checks the result and falls back when it does not agree (`document.md`).

## Limits

- Parsing bounds length (`MAX_LENGTH`, more for stored text), nesting and tree depth (checked as
  each operator is added), so hostile input cannot overflow the stack.
- Evaluation uses an explicit stack, so the deepest stored expression evaluates on a worker's
  default 2 MiB stack in a debug build. Printing, cloning, comparing and dropping recurse but stay
  within it; a test holds this.
