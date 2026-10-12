| scenario | ctl 1f | frames | drops 1f | drops 2f | drops 3f | lat p50 | lat p99 | lat max | jitter p99 | gpu start delay p99 | B fps | B cb p99 ms | VT fps |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| aheavy-baseline | - | 3600 | 0 | 0 | 0 | 8.73 | 9.97 | 12.26 | 1.89 | 1.01 | - | - | - |
| aheavy-sliced-2ms-align | 0 | 3600 | 0 | 0 | 0 | 10.50 | 11.87 | 14.54 | 2.55 | 0.89 | 4.40 | 3.13 | - |
| aheavy-vt-hevc10 | 0 | 3600 | 0 | 0 | 0 | 7.88 | 10.47 | 13.58 | 1.53 | 0.64 | 0.00 | 0.00 | 115.4 |
| aheavy-gate-pause | 0 | 3600 | 2 | 0 | 0 | 10.63 | 12.40 | 21.25 | 2.28 | 0.58 | 3.89 | 3.71 | 115.8 |
