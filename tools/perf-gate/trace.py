"""Read the engine-service lane's opt-in trace without upgrading it to a gate pass."""
import json
import math
from statistics import fmean
from budgets import distribution


def _positive(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value) and value > 0


def onsets(rows):
    """First physical presentation of each content frame, per viewer.

    The service holds an unchanged frame on screen instead of re-presenting it
    (older traces re-presented it every vsync). A held frame's later scanouts
    are not new matches of its media time, so A/V and continuity use onsets.
    """
    shown = sorted((r for r in rows if isinstance(r, dict) and r.get('gpu_ok') is True and _positive(r.get('display'))),
                   key=lambda r: r['display'])
    out, last = [], {}
    for r in shown:
        key = (r.get('time'), r.get('revision'), r.get('generation'))
        if last.get(r.get('viewer_id')) != key:
            out.append(r)
            last[r.get('viewer_id')] = key
    return out


def continuity(rows, frame_ticks, hz, loop_end=None):
    """Dropped content frames: skipped media frames plus onsets later than one
    content interval (+3/4 refresh tolerance for non-multiple cadences)."""
    on = onsets(rows)
    frame_s = frame_ticks / 705_600_000
    late = skipped = 0
    intervals = []
    for a, b in zip(on, on[1:]):
        if a.get('viewer_id') != b.get('viewer_id') or a.get('revision') != b.get('revision'):
            continue
        dt = b['time'] - a['time']
        if dt < 0 and loop_end:
            # Loop wrap: frames between a and the end, and before b, are missing.
            dt = (loop_end - a['time']) + b['time']
            dt = frame_ticks * max(1, round(dt / frame_ticks))
        if dt <= 0:
            continue
        missing = round(dt / frame_ticks) - 1
        skipped += max(0, missing)
        gap = b['display'] - a['display']
        if missing == 0:
            intervals.append(gap * 1000)
            late += gap > frame_s + 0.75 / hz
    return {'content_onsets': len(on), 'skipped_content_frames': skipped, 'late_onsets': late,
            'dropped': skipped + late, 'onset_interval_ms': distribution(intervals)}


def analyze(rows):
    if not isinstance(rows, list):
        raise ValueError('Engine trace must be an array')
    def positive(value):
        return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value) and value > 0
    displayed, work, inputs, offsets = [], [], [], []
    spans = {name: [] for name in [
        "event_to_service_ms", "service_to_worker_ms", "render_ms", "drawable_wait_ms",
        "commit_to_display_ms", "actor_ms", "publication_ms"]}
    for row in rows:
        if not isinstance(row, dict):
            raise ValueError('Malformed engine trace row')
        if row.get('gpu_ok') is not True:
            continue
        display = row.get('display')
        for name, a, b in [
            ('event_to_service_ms', 'input', 'request'),
            ('service_to_worker_ms', 'request', 'worker'),
            ('render_ms', 'render_start', 'render_submitted'),
            ('drawable_wait_ms', 'drawable_asked', 'drawable_acquired'),
            ('commit_to_display_ms', 'commit', 'display')]:
            if positive(row.get(a)) and positive(row.get(b)) and row[b] >= row[a]:
                if name != 'service_to_worker_ms' or positive(row.get('input')):
                    spans[name].append((row[b] - row[a]) * 1000)
        if positive(row.get('input')):
            for name in ['actor_ms', 'publication_ms']:
                if positive(row.get(name)):
                    spans[name].append(row[name])
        if not positive(display):
            continue
        displayed.append(display)
        worker, completed = row.get('worker'), row.get('completed')
        if positive(worker) and positive(completed) and completed >= worker:
            work.append((completed-worker)*1000)
        origin = row.get('input')
        if positive(origin) and origin <= display:
            inputs.append((display-origin)*1000)
        audio = row.get('audio_host')
        if positive(audio):
            # Service contract: audio_host is the device host time of this
            # frame's exact media time. Do not join unrelated clock samples here.
            offsets.append((display-audio)*1000)
    every = offsets
    offsets = [(r['display'] - r['audio_host']) * 1000 for r in onsets(rows) if _positive(r.get('audio_host'))]
    mean = fmean(offsets) if offsets else None
    every_mean = fmean(every) if every else None
    ordered = sorted(displayed)
    return {
        'stages_ms': {name: distribution(values) for name, values in spans.items()},
        'rows':len(rows), 'positive_display_rows':len(displayed),
        'worker_to_completion_ms':distribution(work),
        'input_to_display_ms':distribution(inputs),
        'display_gap_ms':distribution([(b-a)*1000 for a,b in zip(ordered,ordered[1:])]),
        'av_matched_samples':len(offsets),
        'av_mean_signed_offset_ms':mean,
        'av_mean_abs_offset_ms':fmean(map(abs, offsets)) if offsets else None,
        'av_jitter_abs_about_mean_ms':distribution([abs(v-mean) for v in offsets]) if offsets else None,
        'av_basis':'content-frame onsets (first presentation of each media frame)',
        'av_all_presentations':{'samples':len(every),'mean_signed_ms':every_mean,
            'mean_abs_ms':fmean(map(abs, every)) if every else None,
            'jitter_abs_about_mean_ms':distribution([abs(v-every_mean) for v in every]) if every else None},
        'gate_eligible':False,
        'reason':'Native-only diagnostics; trace has no complete-frame census guarantee, paired Electron device stamps, or gate-window markers. Worker span is not GPU-only time.'
    }


def load(path):
    raw = path.read_text()
    try:
        rows = json.loads(raw)
    except json.JSONDecodeError:
        rows = [json.loads(line) for line in raw.splitlines() if line.strip()]
    return analyze([rows] if isinstance(rows, dict) else rows)
