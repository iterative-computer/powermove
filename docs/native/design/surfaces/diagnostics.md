# Diagnostics

Row S115. Grammar: [README](README.md) §1–5. Source: `R/panels/PerfPanel.svelte`,
`R/runtime/performance-monitor.ts`. S053 owns the Error Log and S054 owns status-bar
slowdown details. No new tokens.

## S115 Performance panel

Dockable panel `perf`, title `Performance`, default extent 150 pt. Two-column inspector
rows, read-only, values left-aligned in the shared value column.

v1.2.0's labels name WebGL. Native counters map by what they mean, not by API:

| v1.2.0 | Native label | Value | Native source |
|---|---|---|---|
| `FPS` | `Frame Rate` | `{n} fps`, whole number | presented frames over the last second |
| `Frame ms` | `Frame Time` | `{n} ms`, 2 decimals | CPU plus GPU time for the last frame |
| `GL draws` | `Draw Calls` | count | render-graph draw submissions |
| `FX passes` | `Effect Passes` | count | effect passes this frame |
| `Programs` | `GPU Programs` | count | compiled pipeline states in use |
| `Raster cache` | `Raster Cache` | count | cached raster entries |
| `Layers` | `Layers` | count | current composition |
| `Keyframes` | `Keyframes` | count | current composition |

| State | Spec |
|---|---|
| Live | values update at most twice a second, tabular, fixed width, so numbers never jump |
| Not rendering | `Frame Rate` and `Frame Time` show `—` in `c.muted`; counts remain |
| Counter not reported | `—` in `c.muted`, tooltip `Not available on this Mac.` |
| No project | `EmptyState` `gauge.with.dots.needle.33percent`, `No Project Open`, `Open a project to see its performance.` |
| Hidden | sampling stops while the panel isn't visible, so the panel never costs what it measures |

There is no Pause control: the panel only observes, and v1.2.0 has none. There's no
history graph either. Slowdown attribution stays S054.

Selection: values are selectable, ⌘C copies a row as `{label}: {value}`. Keyboard: rows
are one focus stop with ↑/↓ navigation.

VoiceOver: `AXTable`, rows read `Frame Rate, 60 frames per second`. Updates are not
announced. Increase Contrast and Reduce Motion: nothing specific.

## Don't

- Don't name GL, Metal or any API in visible labels.
- Don't colour values by health; S054 owns that judgement.
