# HDR and camera log

Project Settings › Color › HDR sets `color.mode: "hdr"`. It's off by default, and SDR
projects render as they always have.

| | SDR project | HDR project |
|---|---|---|
| PQ, HLG sources | BT.2390 to SDR white at input | BT.2100 light; 1.0 is SDR white, 203 cd/m² |
| Viewer | SDR | HDR EDR by default; View ▸ Viewer Output also offers SDR, PQ, HLG |
| SDR delivery | as authored | BT.2390 at output, from the content's peak to 203 cd/m² |
| HDR delivery | refused | PQ or HLG, HEVC Main10 or ProRes, with `mdcv` and `clli` |

The working space stays Rec.709 primaries, extended, E-encoded or linear. Wide colours are
negative values, so effects written for 0 to 1 can clip them.

**Tone mapping** runs only where the target holds less light than the frame:
`RenderRequest::output_tone`, in the final encode pass. The viewer's SDR view uses the same
pass as SDR delivery. HDR delivery tone maps only when a source is brighter than
`color.masteringPeak`.

**Metadata.** Mastering primaries and content light come from the sources
(`media::hdr_content`), held to the project's peak. With no stated display, delivery states
Display P3 primaries, D65 and the project's peak. MaxCLL and MaxFALL are written only when
every HDR source states them; they describe the sources, not a measured render.

**Camera log** is an Interpret Footage reading: View ▸ Interpret Selection As. Files don't
carry a tag for these curves, so nothing is detected. A curve reads the code after the
file's range is removed, Resolve's convention; set Range to Full for a camera that writes
codes unscaled. Light above 1.0 stays in the working space. There's no display transform:
an SDR project clips it at output, so use a LUT or an HDR project. Uninterpreted log
footage stays flat, as it does in the shipped app.

**LUTs.** View ▸ Apply LUT… copies the `.cube` into the profile's archive, adds a `lut` asset
and an Apply Color LUT effect on the selected layer, in one Undo step. Input and Output Space
say what the table expects. Working passes values through, which suits a vendor LUT on
uninterpreted log. Saved revision 1 effects keep main's trilinear shader.

**Scopes** are a panel: Window ▸ Panels ▸ Scopes. `pm_render::scopes` bins the viewer's frame
in a compute pass ten times a second at most, only while the panel paints. The standard range
reads display code values. The extended range, an HDR project's default, reads BT.2100 PQ, so
its axis is cd/m² with `SDR White` at 203. The stage is always Output.

**Still refused:** display rotations off the quarter turns, Dolby Vision profile 5 (no base
layer), and primaries with no chromaticities (XYZ).

**Not proven headless:** physical EDR brightness, display moves, QuickTime and Safari
playback of the delivered files.
