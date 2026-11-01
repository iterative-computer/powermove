"""Read the engine-service lane's opt-in trace without upgrading it to a gate pass."""
import json
import math
from statistics import fmean
from budgets import distribution


def analyze(rows):
    if not isinstance(rows, list):
        raise ValueError('Engine trace must be an array')
    def positive(value):
        return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value) and value > 0
    displayed, work, inputs, offsets = [], [], [], []
    for row in rows:
        if not isinstance(row, dict):
            raise ValueError('Malformed engine trace row')
        if row.get('gpu_ok') is not True or not positive(row.get('display')):
            continue
        display = row['display']
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
    mean = fmean(offsets) if offsets else None
    ordered = sorted(displayed)
    return {
        'rows':len(rows), 'positive_display_rows':len(displayed),
        'worker_to_completion_ms':distribution(work),
        'input_to_display_ms':distribution(inputs),
        'display_gap_ms':distribution([(b-a)*1000 for a,b in zip(ordered,ordered[1:])]),
        'av_matched_samples':len(offsets),
        'av_mean_signed_offset_ms':mean,
        'av_mean_abs_offset_ms':fmean(map(abs, offsets)) if offsets else None,
        'av_jitter_abs_about_mean_ms':distribution([abs(v-mean) for v in offsets]) if offsets else None,
        'gate_eligible':False,
        'reason':'Native-only diagnostics; trace has no complete-frame census guarantee, paired Electron device stamps, or gate-window markers. Worker span is not GPU-only time.'
    }


def load(path):
    return analyze(json.loads(path.read_text()))
