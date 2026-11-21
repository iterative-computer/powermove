#!/usr/bin/env python3
"""Prints the per-scene Markdown tables for RESULTS.md from results/summary.csv."""
import csv, os, sys
here = os.path.dirname(os.path.abspath(__file__))
rows = list(csv.DictReader(open(sys.argv[1] if len(sys.argv) > 1 else os.path.join(here, 'results', 'summary.csv'))))
S = {(r['app'], r['scene'], r['metric']): r for r in rows}
AE = 'not run (owner decision)'
# metric, label, lower-is-better
M = [('open_s', 'Open → first rendered frame (s)', True),
     ('playback_fps', 'Playback, rendered fps (of 30)', False),
     ('playback_dropped', 'Playback, frames not rendered (of ~300)', True),
     ('scrub_median_ms', 'Scrub, median per seek (ms)', True),
     ('scrub_p95_ms', 'Scrub, p95 per seek (ms)', True),
     ('export_mp4_s', 'Export H.264 1080p (s)', True),
     ('export_prores422_s', 'Export ProRes 422 (s)', True),
     ('export_prores4444_s', 'Export ProRes 4444 (s)', True),
     ('idle_rss_mb', 'Memory idle after open (MB)', True),
     ('playback_peak_mb', 'Memory peak, playback (MB)', True),
     ('export_mp4_peak_mb', 'Memory peak, H.264 export (MB)', True),
     ('export_prores4444_peak_mb', 'Memory peak, ProRes 4444 export (MB)', True)]
def fmt(v): v = float(v); return f'{v:.0f}' if abs(v) >= 100 else f'{v:.2f}' if abs(v) >= 1 else f'{v:.3f}'
def cell(r): return f"{fmt(r['median'])} ({fmt(r['min'])}–{fmt(r['max'])})" if r else 'n/a'
NA = {('electron', 'export_prores422_s'): 'n/a: v1 exports ProRes 4444 only'}
def table(scene, metrics):
    out = ['| Metric | Electron | Native | AE | Winner | Native ÷ Electron |', '|---|---|---|---|---|---|']
    for m, label, low in metrics:
        e, n = S.get(('electron', scene, m)), S.get(('native', scene, m))
        ec = NA.get(('electron', m)) or cell(e); nc = cell(n)
        win, ratio = '—', '—'
        if e and n:
            ev, nv = float(e['median']), float(n['median'])
            win = 'tie' if ev == nv else ('Native' if (nv < ev) == low else 'Electron')
            ratio = f'{nv / ev:.2f}×' if ev else '—'
        out.append(f'| {label} | {ec} | {nc} | {AE} | {win} | {ratio} |')
    return '\n'.join(out)
print('### Cold launch (all runs)\n')
print(table('all', [('launch_s', 'Cold launch → usable editor window (s)', True)]) + '\n')
for s, name in [('A', 'Scene A — motion graphics'), ('B', 'Scene B — edit'), ('C', 'Scene C — effects stress')]:
    print(f'### {name}\n'); print(table(s, M) + '\n')
