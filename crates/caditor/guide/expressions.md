# Expressions

Every numeric field, sketch dimension and parameter takes an expression: a number with or without
a unit, parameter names, operators and functions.

## Units

- Lengths: `um` (or `µm`), `mm`, `cm` and `m`. Areas and volumes: `mm²`, `mm³` and so on.
- Angles: `deg` (or `°`) and `rad`.

caditor uses SI units only. A plain number typed into a length field means the length unit chosen
in [Preferences](preferences), and into an angle field the angle unit chosen there. Values already
in the model keep the units they were typed in. A decimal is written with a point: `2.5`.

## Operators

- `+`, `-`, `*`, `/` and `^` for powers, with brackets to group: `(a + b) / 2`.
- A unit belongs to the number just before it: `1 / 2 mm` divides by 2 mm; write `(1 / 2) mm` for
  half a millimetre.
- Units carry through: a length times a length is an area, and `sqrt` of an area is a length.
- Comparisons `<`, `<=`, `>`, `>=`, `==` and `!=` give 1 when true and 0 when false. They cannot be
  chained; write `and(a < b, b < c)`.

## Functions

- `sqrt(x)`, `cbrt(x)`, `abs(x)`, `sign(x)`, `exp(x)`, `ln(x)`, `log10(x)`, `log2(x)`.
- `sin`, `cos`, `tan` take an angle; a plain number counts as radians, so write `sin(30 deg)`.
  `asin`, `acos`, `atan` and `atan2(y, x)` give angles.
- `min(a, b, …)`, `max(a, b, …)`, `clamp(x, low, high)`, `hypot(a, b)`, `mod(a, b)`.
- `round(x)`, `floor(x)`, `ceil(x)` and `trunc(x)` round to whole numbers, or to a step:
  `round(x, 0.5 mm)`.
- `if(condition, then, else)`, `and(a, b, …)`, `or(a, b, …)`, `not(a)`.

The constants `pi` (or `π`), `tau` and `e` are known too.

## Names

A [parameter](parameters) name starts with a letter or `_` and holds letters, digits and `_`. It
cannot be a unit, a function or a constant, so `m` or `max` are refused. In a value field,
`name = expression` names the value as a new parameter.

## Typing in a panel

Reaching a value field, by clicking it, with Tab or from {command:palette}, selects its whole
text, so what you type replaces the value. In an open feature's panel the model previews the value
as you type, before you press Enter; when the feature cannot be made with it, the reason shows
under the field.

## Mistakes

An expression that does not read, mixes lengths with angles, divides by zero or uses an unknown
name is refused with the reason under the field; the old value stays until you fix it or press
Escape. An angle field asks for `deg` or `rad` when a calculation gives a plain number, since it
cannot tell which you meant.
