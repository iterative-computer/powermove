#!/usr/bin/env python3
"""Summarize PM_ENGINE_TRACE. Local bounds only; never certifies paired §5 gates."""
import argparse, json, math
from pathlib import Path
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('trace');p.add_argument('--hz',type=int,choices=[60,120],required=True)
a=p.parse_args()
raw=Path(a.trace).read_text()
try: rows=json.loads(raw)
except json.JSONDecodeError: rows=[json.loads(line) for line in raw.splitlines() if line.strip()]
def dist(v):
    v=sorted(v)
    return {'samples':len(v),'p50':v[math.ceil(len(v)*.5)-1] if v else None,'p95':v[math.ceil(len(v)*.95)-1] if v else None}
if isinstance(rows,dict): rows=[rows]
window_path=Path(a.trace[:-11]+'.window.json') if a.trace.endswith('.trace.json') else None
if window_path is not None and window_path.exists():
    window=json.loads(window_path.read_text())
    rows=[r for r in rows if window['start_host']<=r.get('worker',0)<window['end_host']]
else:
    rows=[r for r in rows if r.get('worker',0)>=min((r.get('worker',0) for r in rows),default=0)+1]
positive=sorted(r['display'] for r in rows if r.get('display',0) and r['display']>0)
cached=[(r['display']-r['input'])*1000 for r in rows if r.get('input') and r.get('display',0) and r['display']>=r['input'] and r.get('cache_hit')]
costs=[(r['completed']-r['worker'])*1000 for r in rows if r.get('completed',0)>r.get('worker',0)]
av=[(r['display']-r['audio_host'])*1000 for r in rows if r.get('audio_host') and r.get('display',0) and r['display']>0]
mean=sum(av)/len(av) if av else None
jitter=dist([abs(v-mean) for v in av]) if av else dist([])
print(json.dumps({'physical_gate':'UNVERIFIED: requires review, a full 60 s window, paired Electron/corpus/machine/repetitions',
 'positive_presentations':len(positive),'missing_or_zero_presentations':len(rows)-len(positive),
 'presented_span_seconds':positive[-1]-positive[0] if len(positive)>1 else None,
 'scanout_gaps_over_1_5_intervals':sum(y-x>1.5/a.hz for x,y in zip(positive,positive[1:])) if len(positive)>1 else None,
 'cached_input_to_present_ms':dist(cached),'cached_budget_ms':1000/a.hz+1,
 'worker_to_completion_ms':dist(costs),'headroom_budget_ms':500/a.hz,
 'av_samples':len(av),'av_mean_absolute_ms':sum(abs(v) for v in av)/len(av) if av else None,
 'av_p95_jitter_ms':jitter['p95']},indent=2))
