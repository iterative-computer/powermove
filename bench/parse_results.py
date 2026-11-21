#!/usr/bin/env python3
"""Turns the BENCH/VERIFY log lines of run_all.sh into raw.csv, summary.csv and verify.csv.
Usage: parse_results.py <raw log dir> <results dir>. Summary = median, min, max over runs >= 1 (run 0 = warm-up)."""
import csv, json, os, statistics, sys
raw, res = sys.argv[1], sys.argv[2]; os.makedirs(res, exist_ok=True)
rows = []
def add(app, scene, run, metric, value, unit, note=''):
    rows.append({'app': app, 'scene': scene, 'run': run, 'metric': metric, 'value': value, 'unit': unit, 'note': note})
for app in ['electron', 'native']:
    path = os.path.join(raw, app + '.log')
    if not os.path.exists(path): continue
    launch_scene = None
    for line in open(path):
        if not line.startswith('BENCH '): continue
        v = json.loads(line[6:]); op, run = v['op'], v.get('run')
        scene = os.path.basename(v.get('scene', '')).replace('.pmv', '').replace('scene-', '') or launch_scene
        if op == 'launch': add(app, '', run, 'launch_s', v['launch_s'], 's'); continue
        if op == 'open': add(app, scene, run, 'open_s', v['open_s'], 's', v.get('endpoint', ''))
        elif op == 'idle_rss': add(app, scene, run, 'idle_rss_mb', v['rss_mb'], 'MB')
        elif op == 'playback':
            add(app, scene, run, 'playback_fps', v['fps'], 'fps', 'distinct rendered frames / elapsed; not physical present')
            add(app, scene, run, 'playback_dropped', v['dropped'], 'frames', f"of {v['expected']} expected")
            add(app, scene, run, 'playback_peak_mb', v['mem']['peak_mb'], 'MB')
        elif op == 'scrub':
            add(app, scene, run, 'scrub_median_ms', v['median_ms'], 'ms'); add(app, scene, run, 'scrub_p95_ms', v['p95_ms'], 'ms')
        elif op == 'export':
            f = v['format']
            if v.get('error') or not v.get('path'): add(app, scene, run, f'export_{f}_s', '', 's', 'error: ' + str(v.get('error'))[:200]); continue
            add(app, scene, run, f'export_{f}_s', v['export_s'], 's'); add(app, scene, run, f'export_{f}_peak_mb', v['mem']['peak_mb'], 'MB')
        elif op == 'failed': add(app, scene.replace('.pmv', ''), run, 'failed', v.get('exit'), 'exit', 'run crashed or timed out')
# Launch rows: attribute to the scene that run opened (each run is a fresh launch).
for i, r in enumerate(rows):
    if r['metric'] == 'launch_s':
        nxt = next((x for x in rows[i + 1:] if x['app'] == r['app'] and x['metric'] == 'open_s'), None)
        r['scene'] = nxt['scene'] if nxt else ''
with open(os.path.join(res, 'raw.csv'), 'w', newline='') as fh:
    w = csv.DictWriter(fh, fieldnames=list(rows[0].keys())); w.writeheader(); w.writerows(rows)
groups = {}
for r in rows:
    if r['run'] is None or r['run'] < 1 or r['value'] in ('', None) or r['metric'] == 'failed': continue
    key = (r['app'], 'all' if r['metric'] == 'launch_s' else r['scene'], r['metric']); groups.setdefault(key, []).append(float(r['value']))
with open(os.path.join(res, 'summary.csv'), 'w', newline='') as fh:
    w = csv.writer(fh); w.writerow(['app', 'scene', 'metric', 'n', 'median', 'min', 'max'])
    for k in sorted(groups): x = groups[k]; w.writerow([*k, len(x), round(statistics.median(x), 4), round(min(x), 4), round(max(x), 4)])
vp = os.path.join(raw, 'verify.log')
if os.path.exists(vp):
    with open(os.path.join(res, 'verify.csv'), 'w', newline='') as fh:
        w = csv.writer(fh); w.writerow(['file', 'codec', 'profile', 'pix_fmt', 'width', 'height', 'frames', 'duration_s', 'audio'])
        for line in open(vp):
            if not line.startswith('VERIFY '): continue
            v = json.loads(line[7:]); s = v['probe']['streams'][0]
            w.writerow([v['file'], s.get('codec_name'), s.get('profile'), s.get('pix_fmt'), s.get('width'), s.get('height'), s.get('nb_read_frames'), v['probe']['format']['duration'], v['audio']])
print('rows', len(rows), 'groups', len(groups))
