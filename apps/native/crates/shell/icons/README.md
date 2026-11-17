# Icons

Glyphs the app draws in place of an SF Symbol of the same meaning (`src/icons.rs`
maps symbol names to these files). They are compiled into the binary; nothing is
fetched at run time.

## `pierre/`

From Pierre's icon set: <https://github.com/pierrecomputer/icons>, npm
`@pierre/icons` 0.9.0 (commit 1c93d61bb031, 2026-09-16), licensed under the Apache
License 2.0 (`pierre/LICENSE`). The files are copied unchanged. Where the app shows a
glyph turned a quarter (align top, middle, bottom are align left, centre, right
turned 90°), the turn is applied when it is drawn, not to the file.

The Pierre name and logo are not used as ours; `IconPierre` and the `IconBrand*`
marks are not included.

## `powermove/`

Glyphs drawn for Powermove where the Pierre set has none, on the same 16 px grid,
in the same weights (1.5 px lines with round ends, 5 px bars with 1.5 px corners).

- `distribute-horizontal.svg`: two edges and the object between them at equal gaps;
  distribute vertically is the same glyph turned a quarter.
