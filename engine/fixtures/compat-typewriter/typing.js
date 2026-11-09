// Pure text/time logic. No editor state, project writes or playback timers.
const segmenter = new Intl.Segmenter(undefined, { granularity: 'grapheme' });

export function graphemes(text) {
  return Array.from(segmenter.segment(String(text)), part => part.segment);
}

export function typedCount(progress, length) {
  const p = Number.isFinite(Number(progress)) ? Number(progress) : 0;
  return Math.min(length, Math.max(0, Math.floor(Math.min(100, p) * length / 100 + 1e-8)));
}

export function typedState(text, progress) {
  const units = graphemes(text);
  const count = typedCount(progress, units.length);
  return { count, length: units.length, text: units.slice(0, count).join(''), complete: count === units.length };
}

// Frame-addressable blinking also works when scrubbing backwards or exporting
// out of order. No last-frame state or wall-clock animation is involved.
export function cursorVisible(time, from, fps, state, settings, countAt) {
  if (!settings.cursor || (settings.hideOnComplete && state.complete)) return false;
  const rate = Math.max(0, Number(settings.blinkRate) || 0);
  if (!rate) return true;
  const delay = Math.max(0, Number(settings.blinkDelay ?? 0.5) || 0);
  const frame = 1 / Math.max(1, fps);
  const window = delay + 1 / rate;
  const frames = Math.min(1800, Math.ceil(window / frame));
  let idle = Math.max(0, time - from);
  for (let i = 1; i <= frames && time - i * frame >= from - 1e-8; i++) {
    if (countAt(time - i * frame) !== state.count) {
      idle = Math.max(0, (i - 1) * frame);
      break;
    }
  }
  return idle <= delay || ((Math.max(0, time - from) - delay) * rate) % 1 < 0.5;
}

export function gradientPosition(t, offset, cycle, time) {
  const position = t + offset / 100 + cycle * time;
  if (!cycle) return Math.min(1, Math.max(0, position));
  const phase = ((position % 2) + 2) % 2;
  return phase <= 1 ? phase : 2 - phase;
}

export function gradientColor(t, colors, middle) {
  const m = Math.min(0.99, Math.max(0.01, middle / 100));
  const a = t < m ? colors[0] : colors[1];
  const b = t < m ? colors[1] : colors[2];
  const f = t < m ? t / m : (t - m) / (1 - m);
  return a.map((v, i) => Math.round(v + (b[i] - v) * f));
}
