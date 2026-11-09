# Media Offline

Powermove's sign for missing media: one look in the viewer, agent stills,
the timeline and the Project panel, readable from a thumbnail to full screen.
Taste reference only: Premiere's red card. Ours is built from our tokens.

## The card (viewer and agent stills)

Drawn by the engine in the layer's own box (its `d.w × d.h`, else the
composition size), under the layer's transform, opacity and effects, so it
sits exactly where the footage would. It is drawn over the image, so it uses
the dark-scheme tokens in both appearances (language §4 *Over the image*).

- **Field.** `colors.viewer` (`#0E0E0E`) with 45° hatch stripes in
  `colors.danger` (`#F36357`) at 18 %: the recognisable part. It reads as
  "something is missing here" even at thumbnail size, where no text can.
- **Plate.** A centred rounded plate on `colors.viewer` at 94 % with a `text`
  hairline (8 %), holding, top to bottom:
  - a `danger` disc with a white exclamation mark;
  - **Media Offline**: SF Pro semibold, `colors.text`;
  - the file name: regular, `colors.secondary`, cut with an ellipsis;
  - `Locate File… in the Project panel`: regular, `colors.muted`.
- **Scale.** Type sizes follow the box (title ≈ 8 % of its width, at most
  14 % of its height). The layout steps by the box's size on screen:
  under 120 px high only the hatch, disc and title show; under 260 px the
  file name joins; above that, the hint too. The card is drawn at output
  resolution, so it stays crisp in a small viewer and full screen.
- **Never in exports.** Only frames that isolate failing layers (the viewer,
  agent stills) draw it. Exports keep refusing and name the missing media.
  A layer that draws a card is not reported as "Not drawn": the card names
  the file.

Relink: the Project row's hover action and right-click menu, **Locate File…**
(already there); the row reads `Missing` in `danger` with the path in its
tooltip.

## Missing extension effect

The layer still draws with the effect bypassed. No card: the timeline clip
and the Effect Controls row carry the sign (a `danger` exclamation and
`Missing effect`), never the picture.

## Timeline

Clips of missing media (video, image and audio-only layers) use the same
sign: the clip keeps its shape and name, its fill becomes `colors.viewer` with
the `danger` hatch, its name is followed by `Offline` in `danger`. Audio-only
layers have no picture to replace, so this lane treatment is their sign.
