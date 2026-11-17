# Icons

Glyphs the app draws in place of an SF Symbol of the same meaning (`src/icons.rs`
maps symbol names to these files). They are compiled into the binary; nothing is
fetched at run time. SF Symbols remain where AppKit itself draws the glyph inside a
control (pop-up chevrons, switches, menu marks, the window buttons) and for symbols
not mapped yet. The Hand tool and Export use Apple's own filled drawings
(`hand.raised.fill`, and `square.and.arrow.up` at semibold), drawn at the vendored
glyphs' size.

## `pierre/`

From Pierre's icon set: <https://github.com/pierrecomputer/icons>, npm
`@pierre/icons` 0.9.0 (commit 1c93d61bb031, 2026-09-16), licensed under the Apache
License 2.0 (`pierre/LICENSE`). The files are copied unchanged. The filled variant is
used wherever the set has one (`IconHomeFill`, `IconGearFill`, `IconBinFill`, …; the shy
switch uses `IconEmojiSmileFill` in both states, as its dashed twin reads as a sun at 1x);
icons it draws only in line form (`IconEye`, `IconLock`, `IconLink`, …) are used as
they are. Turns and mirrors (align top, middle and bottom are align left, centre and
right turned 90°; the counter-clockwise arrow is `IconReload` mirrored) are applied
when a glyph is drawn, not to the files.

The Pierre name and logo are not used as ours; `IconPierre` and the `IconBrand*`
marks are not included. The app's About window carries the notice
(`apps/native/bundle/Credits.rtf`) and the bundle ships the licence text.

## `powermove/`

Glyphs drawn for Powermove where the Pierre set has none (transport, audio and solo
switches, tools, layer kinds, keyframe marks), on the same 16 px grid and in the same
grammar: filled shapes with 1.5 px corner radii, 1.5 px lines with round ends, 5 px
bars, knockouts by mask, no thinner strokes or outlines.

- `distribute-horizontal.svg`: Pierre's own edge line at each side (the
  `IconAlignLeft` / `IconAlignRight` paths) and one 5 px bar between them at equal
  gaps; distribute vertically is the same glyph turned a quarter. Two bars fit only
  with 1 px gaps and read as one block at 1x.
- `solo.svg` / `solo-off.svg`: After Effects' solo switch, a dot: larger and in the accent
  while on, small and dim while off (the timeline tints them).
- `pen.svg`: the Pen tool's nib (filled, with its slit and point); Pierre's pencil means
  rename.
- `plus.svg`: drawn full size in a 2 px line; Pierre's plus (10 of 16 px, 1.5 px) reads
  small beside the search glass.
