# HDR and camera log

Project Settings › Color › HDR sets `color.mode: "hdr"`. It's off by default, and SDR
projects render as they always have.

| | SDR project | HDR project |
|---|---|---|
| PQ, HLG sources | BT.2390 to SDR white at input | BT.2100 light; 1.0 is SDR white, 203 cd/m² |
| Viewer | SDR | HDR EDR by default, the HDR delivery's picture; View ▸ Viewer Output also offers SDR, PQ, HLG |
| SDR delivery | as authored | BT.2390 at output, from the content's peak to 203 cd/m² |
| HDR delivery | refused | PQ or HLG, HEVC Main10 or ProRes, with `mdcv` and `clli` |

The working space stays Rec.709 primaries, extended, E-encoded or linear. Wide colours are
negative values, so effects written for 0 to 1 can clip them.

**Tone mapping** is `OutputTone::for_target`, in the final encode pass; the viewer's SDR and
EDR views and the Output scopes use the same pass and rule as SDR and HDR delivery. An HDR
target always holds `color.masteringPeak`: content stated brighter rolls off to it (BT.2390),
and light a grade lifts past it is held at it, each channel at most the master. SDR targets
tone map when the content or the master is brighter than SDR white. The content's peak comes
from what the composition can show: switched-off layers and unused compositions don't count.
Camera log has no stated peak, so a shown log clip counts as what its curve can hold (S-Log3
about 7800 cd/m²).

**Metadata.** The mastering display is the project's master: its peak, with Display P3
primaries and D65, whatever the sources state. MaxCLL and MaxFALL are measured from the frames
the export renders, after the tone: the most of each frame's brightest largest-of-R, G and B,
and of each frame's average of it, in cd/m² (`VerifyReport::content_light`). They are written
into the file's `clli` once every frame is encoded, and the file check holds the file to them.
macOS writes `mdcv` and `clli` in the container only, not as SEI in the stream.

**Camera log** is an Interpret Footage reading: View ▸ Interpret Selection As. Files don't
carry a tag for these curves, so nothing is detected. A curve reads the code after the
file's range is removed, Resolve's convention; set Range to Full for a camera that writes
codes unscaled. Light above 1.0 stays in the working space. There's no display transform:
an SDR project clips it at output, so use a LUT or an HDR project. Uninterpreted log
footage stays flat, as it does in the shipped app. LogC3 is ARRI's EI 800 curve, and the menu
says so; footage shot at another EI decodes slightly off.

**LUTs.** View ▸ Apply LUT… copies the `.cube` into the profile's archive, adds a `lut` asset
and fills the selected layer's empty Apply Color LUT or adds one, in one Undo step. A file is
read when its stamp changes and its table uploads once. Input and Output Space
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
