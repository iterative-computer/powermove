"""P-120 evidence parser. Zero/missing/duplicate timestamps never count as a pass."""
import math
import re

def percentile(v, q):
    return sorted(v)[min(len(v)-1, math.ceil(q*len(v))-1)]
def metrics(path):
    content = path.read_text()
    cpu = [float(n)*{'ns': .000001, 'µs': .001, 'ms': 1, 's': 1000}[u]
           for n,u in re.findall(r'frame duration: ([\d.]+)(ns|µs|ms|s)', content)][30:]
    thread_cpu = [float(n) for n in re.findall(r'PM_GPUI_MAIN_CPU_MS ([\d.]+)', content)][30:]
    timestamps = [float(n) for n in re.findall(r'PM_GPUI_PRESENT ([\d.]+)', content)][30:]
    # Main-thread commit→waitUntilScheduled time; only transactional presents log it.
    waits = [float(n)*{'ns': .000001, 'µs': .001, 'ms': 1, 's': 1000}[u]
             for n,u in re.findall(r'GPUI scheduled: ([\d.]+)(ns|µs|ms|s)', content)][30:]
    # Main-thread nextDrawable time (drawable-pool starvation shows up here).
    drawable = [float(n)*{'ns': .000001, 'µs': .001, 'ms': 1, 's': 1000}[u]
                for n,u in re.findall(r'GPUI drawable: ([\d.]+)(ns|µs|ms|s)', content)][30:]
    times = sorted(t for t in timestamps if t > 0)
    gaps = [b-a for a,b in zip(times,times[1:])]
    # Zero timestamps are unknown, never counted as successful presentations.
    dropped = sum(max(0, round(d*120)-1) for d in gaps)
    valid = (len(cpu) >= 6000 and len(times) >= len(cpu) and 0 not in timestamps
             and len(times) > 1 and times[-1]-times[0] >= 60 and all(d>0 for d in gaps))
    return {'cpu_samples':len(cpu),'deadline_skips':content.count('PM_GPUI_SKIP_DEADLINE'),
            'main_thread_cpu_p50_ms':percentile(thread_cpu,.5) if thread_cpu else None,
            'main_thread_cpu_p95_ms':percentile(thread_cpu,.95) if thread_cpu else None, 'cpu_p50_ms':percentile(cpu,.5) if cpu else None,
            'cpu_p95_ms':percentile(cpu,.95) if cpu else None,
            'cpu_p99_ms':percentile(cpu,.99) if cpu else None,
            'scheduled_waits':len(waits),
            'scheduled_wait_p50_ms':percentile(waits,.5) if waits else 0,
            'scheduled_wait_p95_ms':percentile(waits,.95) if waits else 0,
            'drawable_wait_p50_ms':percentile(drawable,.5) if drawable else None,
            'drawable_wait_p95_ms':percentile(drawable,.95) if drawable else None,
            'gap_p50_ms':percentile(gaps,.5)*1000 if gaps else None,
            'gap_p95_ms':percentile(gaps,.95)*1000 if gaps else None,
            'gap_max_ms':max(gaps)*1000 if gaps else None,
            'positive_presentations':len(times),'unknown_presentations':timestamps.count(0),
            'missed_120hz_intervals':dropped,'presentation_evidence_valid':valid,
            'tile_render_calls':content.count('S2_TILE_RENDER'),
            'pass':bool(valid and dropped==0 and percentile(cpu,.95)<=1000/120/2)}
