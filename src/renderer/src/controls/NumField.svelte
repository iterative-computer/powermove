<script lang="ts">
  import { sel } from '../state/selection.svelte';
  import { tick, onDestroy } from 'svelte';
  import { doc } from '../state/document.svelte';
  import { transport } from '../state/transport.svelte';
  import { parseArithmetic } from './arith';
  import { EditGesture, type EditBinding } from './gesture';
  import { rowLabelId } from './context';
  import { round } from './control-utils';
  import './controls.css';

  let {
    PM,
    get,
    edit,
    min,
    max,
    step,
    speed,
    unit = '',
    precision,
    label,
    ariaLabel,
    link = false,
    onInput,
    onCommit
  }: {
    PM: Record<string, any>;
    get: () => unknown;
    edit: EditBinding;
    min?: number;
    max?: number;
    step?: number;
    speed?: number;
    unit?: string;
    precision?: number;
    label?: string;
    ariaLabel?: string;
    link?: boolean;
    onInput?: (value: number) => void;
    onCommit?: (value: number) => void;
  } = $props();

  const labelledBy = rowLabelId();
  const value = $derived((doc.tick.values, doc.proj, transport.time, get()));
  const numeric = $derived(typeof value === 'number' ? value : Number(value));
  const gesture = $derived(new EditGesture(PM, edit));
  let input: HTMLInputElement;
  let editing = $state(false);
  let draft = $state('');
  const effectiveStep = $derived(step || 1);
  const effectiveSpeed = $derived(speed || 0.5);

  const format = (inputValue: unknown): string => {
    if (typeof inputValue !== 'number' || !Number.isFinite(inputValue)) return String(inputValue);
    const places = precision ?? (step && step < 1 ? 2 : (Math.abs(inputValue) < 10 ? 1 : 0));
    let result = inputValue.toFixed(places);
    if (places > 0) result = result.replace(/\.?0+$/, '');
    return result + unit;
  };

  const mixed = $derived((sel.layers,doc.tick.values, doc.tick.structure, doc.proj, transport.time, PM.inspectorMixed?.(edit,value) ?? false));
  const shown = $derived(editing ? draft : mixed ? 'Mixed' : format(value));

  function openEditor(): void {
    draft = String(round(PM, Number(get()), 3));
    editing = true;
    void tick().then(() => { input.focus(); input.select(); });
  }

  function parseDraft(): number {
    const raw = draft.trim();
    return parseArithmetic(/^[-+*/]/.test(raw) ? `${Number(get())}${raw}` : raw);
  }

  function clampValue(next: number): number {
    if (min != null) next = Math.max(min, next);
    if (max != null) next = Math.min(max, next);
    return round(PM, next, 4);
  }

  function finish(commit: boolean): void {
    if (!editing) return;
    if (commit) {
      const next = parseDraft();
      if (Number.isFinite(next)) {
        const rounded = round(PM, next, 4);
        gesture.once(rounded);
        onCommit?.(rounded);
      }
    }
    editing = false;
    draft = '';
  }

  let cancelScrub: (() => void) | undefined;
  onDestroy(() => cancelScrub?.());

  function pointerdown(event: PointerEvent): void {
    if (editing || event.button !== 0) return;
    event.preventDefault();
    const start = Number(get());
    let moved = false;
    const scrub = gesture;
    let active = true;
    const cleanup = () => { active = false; window.removeEventListener('keydown', escape, true); cancelScrub = undefined; };
    const cancel = () => { if (!active) return; cleanup(); scrub.cancel(); };
    const escape = (key: KeyboardEvent) => {
      if (key.key !== 'Escape') return;
      key.preventDefault(); key.stopImmediatePropagation();
      handle?.cancel(); cancel();
    };
    cancelScrub = () => { handle?.cancel(); cancel(); };
    window.addEventListener('keydown', escape, true);
    scrub.begin();
    const handle = PM.drag(event, {
      cursor: 'ew-resize',
      move: (dx: number, _dy: number, nextEvent: PointerEvent) => {
        if (!active || (!moved && Math.abs(dx) < 3)) return;
        moved = true;
        const multiplier = nextEvent.shiftKey ? 10 : nextEvent.altKey ? 0.1 : 1;
        let next = start + dx * effectiveStep * multiplier * effectiveSpeed;
        if (min != null) next = Math.max(min, next);
        if (max != null) next = Math.min(max, next);
        next = round(PM, next, 3);
        scrub.write(next);
        onInput?.(next);
      },
      up: () => {
        if (!active) return;
        cleanup();
        if (!moved) { openEditor(); scrub.cancel(); }
        else { scrub.commit(); onCommit?.(Number(get())); }
      },
      cancel
    });
  }

  function keydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (!editing) {
      if (event.key === 'Enter' || event.key === 'F2') {
        event.preventDefault();
        openEditor();
      } else if (event.key === 'ArrowUp' || event.key === 'ArrowDown') {
        event.preventDefault();
        const direction = event.key === 'ArrowUp' ? 1 : -1;
        const next = clampValue((Number.isFinite(numeric) ? numeric : 0) + direction * effectiveStep * (event.shiftKey ? 10 : 1));
        gesture.once(next);
        onCommit?.(next);
      }
      return;
    }
    if (event.key === 'Enter') {
      event.preventDefault();
      finish(true);
      input.blur();
    } else if (event.key === 'Escape') {
      event.preventDefault();
      finish(false);
    } else if (event.key === 'ArrowUp' || event.key === 'ArrowDown') {
      event.preventDefault();
      const direction = event.key === 'ArrowUp' ? 1 : -1;
      const parsed = Number.parseFloat(draft || '0');
      const next = (Number.isFinite(parsed) ? parsed : 0) + direction * effectiveStep * (event.shiftKey ? 10 : 1);
      draft = String(round(PM, next, 4));
    }
  }
</script>

<input
  bind:this={input}
  class="num"
  class:editing
  class:link
  type="text"
  inputmode="decimal"
  role="spinbutton"
  readonly={!editing}
  value={shown}
  title={label || ''}
  aria-labelledby={ariaLabel ? undefined : labelledBy}
  aria-label={ariaLabel ?? (labelledBy ? undefined : (label ?? edit.label))}
  aria-valuenow={Number.isFinite(numeric) ? numeric : undefined}
  aria-valuemin={min}
  aria-valuemax={max}
  onpointerdown={pointerdown}
  onfocus={() => { if (!editing) openEditor(); }}
  oninput={(event) => { if (editing) draft = event.currentTarget.value; }}
  onblur={() => finish(true)}
  onkeydown={keydown}
/>
