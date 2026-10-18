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
  one `skrifa` `Location` per weight, the `wght` axis clamped to the face's range, GPOS `kern` pair
  kerning (Latin, else the default script), advances, cap height (`Metrics::cap_height`) and
  outlines. A glyph's outline is cached per weight and glyph as `Segment`s (lines and quadratic or
  cubic Béziers, as drawn) after merging.
- A variable font keeps overlapping contours (Inter's `e`, `t`, `f`, `k`), which even-depth regions
  would read as notches, so `overlap.rs` merges each glyph first (`merged`): its segments build a
  kernel `Profile`, faces whose anchor has non-zero winding over the original contours (Béziers
  sampled 32 steps) are selected as lumps, and each loop's pieces become lines and Béziers (de
  Casteljau) meeting exactly at junctions, zero-length pieces dropped. A glyph the profile cannot
  build keeps its contours as drawn. Different glyphs that overlap are not merged.
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
