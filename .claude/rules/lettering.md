---
paths:
  - "crates/caditor-lettering/**"
---

# Lettering (`caditor-lettering`)

Letter outlines in Inter, shared by SVG text import (`file-import-export.md`) and sketch text
(`sketch.md`, `document.md`). It depends on `caditor-geometry` and `caditor-kernel` (overlap
merging builds a kernel `Profile`) and `skrifa`, so both `caditor-file` and `caditor-document` use
it without either depending on the other.

- The crate carries Inter's upright and italic faces (`INTER`, `INTER_ITALIC`, included from
  `crates/caditor/assets/fonts/`); the app's `fonts.rs` takes them from here, so the binary holds
  one copy. SVG import still has the faces handed in (`TextOutlines`), the app passing these.
- `Typeface` (`font.rs`) reads a face pair: one `Cut` per face, an `Instance` of weight and italic,
  one `skrifa` `Location` per weight, the `wght` axis clamped to the face's range and rounded to the nearest hundred (`WEIGHT_STEP`), so
  the advances, metrics and outlines of a weight are those of its hundred and any whole weight from
  101 to 899 shares at most nine cached outlines per glyph, GPOS `kern` pair
  kerning (Latin, else the default script), advances, cap height (`Metrics::cap_height`), the
  OS/2 subscript and superscript offsets (`Typeface::subscript_drop`, `superscript_rise`, font
  units, `None` when the face gives zero) and outlines. A glyph's outline is cached per weight and glyph as `Segment`s (lines and quadratic or
  cubic Béziers, as drawn) after merging.
- A variable font keeps overlapping contours (Inter's `e`, `t`, `f`, `k`), which even-depth regions
  would read as notches, so `overlap.rs` merges each glyph first (`merged`): its segments build a
  kernel `Profile`, faces whose anchor has non-zero winding over the original contours (Béziers
  sampled 32 steps) are selected as lumps, and each loop's pieces become lines and Béziers (de
  Casteljau) meeting exactly at junctions, zero-length pieces dropped. A glyph the profile cannot
  build keeps its contours as drawn; a merge the kernel interrupt cancels (`merged` polls it first
  and returns `Interrupted`) is not cached and ends `Typeface::outline`, `lettered`
  (`LetteringError::Cancelled`) and the SVG walk (`ImportError::Cancelled`) there. Different glyphs that overlap are not merged.
- `lettered(content, Style, Alignment)` (`layout.rs`) lays text out in the bundled Inter, held in a
  `LazyLock` behind a `parking_lot::Mutex` so recompute threads share the glyph cache: weight 400,
  or 700 when bold, the italic face when italic. Units are cap heights: the first line's baseline
  is y = 0 and a capital stands 1 tall, so a sketch scales by its height alone. Lines split at
  `\n`; each is laid out from x = 0 (the pen moves by advance plus kerning between neighbours;
  control characters are skipped, whitespace advances and draws nothing, and a character the face
  lacks takes the missing glyph's advance and is listed in `Lettered::missing`), then shifted left
  by none (`Alignment::Left`), half or all of its own advance width (`Centre`, `Right`, trailing
  spaces counting, so the anchor sits at the start, middle or end of the line as typed), and down
  by its index times `Lettered::line_pitch`: Inter's ascent minus descent plus line gap over its
  cap height (about 1.66; 1.6 if the face reports none), read by `Typeface::line_height`. A blank
  line takes a pitch of height and no width. `Lettered::line_widths` holds every line's advance
  width and `width` the widest. Each `Letter` keeps its character position in the whole string,
  line breaks counting as one, which sketch text names its curves by, so a letter's name does not
  change when a line is added after it, and the first line's letters keep their names whatever
  follows.
- Inter has no diameter sign, depth, counterbore or countersink (U+2300 ⌀, U+21A7 ↧, U+2334 ⌴,
  U+2335 ⌵), which drawings write in a dimension's value and a callout, so `signs.rs` draws them
  itself: in cap heights, from a stroke width, as ring arcs (Bézier circles) and mitred polylines
  closed into outlines, then merged like a glyph. `Cut::sign` scales a design to the face, takes
  its stroke from the width of Inter's `I` at the weight (a heavier weight draws heavier signs,
  `FALLBACK_STEM` when the face has no `I`), shears it by the italic cut's `post` angle, and caches
  it per weight and character beside the glyph outlines. `lay_line` asks for a sign only for a
  character the face lacks, so it is never listed in `Lettered::missing`; it advances by the
  design's width plus a side bearing, takes no kerning and is a `Letter` like any other. `Typeface::sign` is
  the one source: SVG import asks it too for a character without a glyph (`file-import-export.md`).
