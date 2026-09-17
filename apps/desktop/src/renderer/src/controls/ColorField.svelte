<script lang="ts">
  /* Colour popover. Anatomy after dialkit's colour control: a plane, a hue
     track, one value field with a format switch, and a row of swatches. It is
     a menu, not a dialog: no header, no buttons, click outside commits, Escape
     cancels, and it opens and closes with the dropdown menu's motion. */
  import { sel } from '../state/selection.svelte';
  import { tick } from 'svelte';
  import { doc } from '../state/document.svelte';
  import { transport } from '../state/transport.svelte';
  import { EditGesture, type EditBinding } from './gesture';
  import { rowLabelId } from './context';
  import { clamp, hexToRgb, hsvToRgb, normalizeHex, rgbToHex, rgbToHsv } from './control-utils';
  import { anchorPicker, mountOverlayOnBody } from './overlay';
  import './controls.css';
  import type { PowermoveAPI } from '../kernel/api';

  let {
    api,
    get,
    edit,
    label = 'Color',
    mixed
  }: {
    api: PowermoveAPI;
    get: () => unknown;
    edit: EditBinding;
    label?: string;
    mixed?: (edit: EditBinding, value: unknown) => boolean;
  } = $props();

  type Format = 'hex' | 'rgb' | 'hsb';
  type EyeDropperConstructor = new () => { open(): Promise<{ sRGBHex: string }> };

  const labelledBy = rowLabelId();
  const raw = $derived((doc.tick.values, doc.proj, transport.time, get()));
  const value = $derived(typeof raw === 'string' && /^#[0-9a-f]{3,8}$/i.test(raw) ? raw : '#808080');
  const isMixed = $derived((sel.layers, doc.tick.values, doc.proj, transport.time, mixed?.(edit, value) ?? false));
  const gesture = $derived(new EditGesture(api, edit));
  const presets = ['#09090A', '#FFFFFF', '#FF6B1A', '#FFB000', '#34C759', '#0A84FF', '#6E5AE6', '#FF375F'];
  const formats: Array<[Format, string]> = [['hex', 'Hex'], ['rgb', 'RGB'], ['hsb', 'HSB']];

  let trigger = $state<HTMLButtonElement>();
  let popover = $state<HTMLDivElement>();
  let valueInput = $state<HTMLInputElement>();
  let open = $state(false);
  let phase = $state<'open' | 'closed'>('open');
  let format = $state<Format>('hex');
  /* `chosen` stays authoritative so typed values never drift through an HSV round trip. */
  let chosen = $state('#808080');
  let before = $state('#808080');
  let hsv = $state({ h: 0, s: 0, v: 50 });
  let draft = $state('#808080');
  let invalid = $state(false);
  let previewing = $state(false);
  let sampling = $state(false);
  let closeTimer: number | undefined;

  const hueColor = $derived(rgbToHex(hsvToRgb({ h: hsv.h, s: 100, v: 100 })));
  const formatIndex = $derived(formats.findIndex(([id]) => id === format));
  const inputLabel = $derived(`${label} ${format === 'hex' ? 'hex' : format.toUpperCase()} value`);

  function formatted(hex: string, as: Format): string {
    if (as === 'hex') return hex;
    const rgb = hexToRgb(hex) ?? { r: 0, g: 0, b: 0 };
    if (as === 'rgb') return `${rgb.r}, ${rgb.g}, ${rgb.b}`;
    const next = rgbToHsv(rgb);
    return `${next.h}°, ${next.s}%, ${next.v}%`;
  }

  function parse(text: string, as: Format): string | null {
    const trimmed = text.trim();
    if (/^#?[0-9a-f]{6}$/i.test(trimmed) || /^#[0-9a-f]{3}$/i.test(trimmed)) return normalizeHex(trimmed);
    if (as === 'hex') return null;
    const numbers = trimmed.match(/-?\d+(?:\.\d+)?/g)?.map(Number);
    if (!numbers || numbers.length !== 3) return null;
    const [a, b, c] = numbers as [number, number, number];
    return as === 'rgb' ? rgbToHex({ r: a, g: b, b: c }) : rgbToHex(hsvToRgb({ h: a, s: b, v: c }));
  }

  function commitHex(next: unknown): boolean {
    const valid = normalizeHex(next);
    if (!valid) return false;
    chosen = valid;
    hsv = rgbToHsv(hexToRgb(valid)!);
    return true;
  }

  function previewChosen(): void {
    if (!previewing) {
      gesture.begin();
      previewing = true;
    }
    gesture.write(chosen);
    api.transport.invalidate('render');
  }

  function setHex(next: unknown, preview = true): boolean {
    if (!commitHex(next)) return false;
    draft = formatted(chosen, format);
    invalid = false;
    if (preview) previewChosen();
    return true;
  }

  function typeDraft(): void {
    invalid = false;
    const parsed = parse(draft, format);
    if (parsed && commitHex(parsed)) previewChosen();
  }

  function settleDraft(): void {
    const parsed = parse(draft, format);
    if (parsed) setHex(parsed);
    else invalid = true;
  }

  function switchFormat(next: Format): void {
    format = next;
    draft = formatted(chosen, format);
    invalid = false;
  }

  function formatKey(event: KeyboardEvent): void {
    const delta = event.key === 'ArrowRight' || event.key === 'ArrowDown' ? 1 : event.key === 'ArrowLeft' || event.key === 'ArrowUp' ? -1 : 0;
    if (!delta) return;
    event.preventDefault();
    const next = formats[(formatIndex + delta + formats.length) % formats.length]!;
    switchFormat(next[0]);
    void tick().then(() => popover?.querySelector<HTMLButtonElement>('.cp-format[aria-checked="true"]')?.focus());
  }

  async function sampleScreenColor(): Promise<void> {
    const EyeDropper = (window as Window & { EyeDropper?: EyeDropperConstructor }).EyeDropper;
    if (!EyeDropper) {
      api.ui.toast('Eyedropper is not available on this system');
      return;
    }
    if (sampling) return;
    sampling = true;
    try {
      const result = await new EyeDropper().open();
      setHex(result.sRGBHex);
    } catch (error) {
      if (!(error instanceof DOMException && error.name === 'AbortError')) api.ui.toast('Could not sample that color');
    } finally {
      sampling = false;
    }
  }

  function fromHsv(next: { h: unknown; s: unknown; v: unknown }): void {
    hsv = {
      h: Math.round(clamp(Number(next.h) || 0, 0, 359)),
      s: Math.round(clamp(Number(next.s) || 0, 0, 100)),
      v: Math.round(clamp(Number(next.v) || 0, 0, 100))
    };
    chosen = rgbToHex(hsvToRgb(hsv));
    draft = formatted(chosen, format);
    invalid = false;
    previewChosen();
  }

  function pickSv(event: PointerEvent, element: HTMLElement): void {
    const rect = element.getBoundingClientRect();
    if (!rect.width || !rect.height) return;
    fromHsv({ h: hsv.h, s: (event.clientX - rect.left) / rect.width * 100, v: 100 - (event.clientY - rect.top) / rect.height * 100 });
  }

  function pickHue(event: PointerEvent, element: HTMLElement): void {
    const rect = (element.querySelector('.cp-hue-track') ?? element).getBoundingClientRect();
    if (!rect.width) return;
    fromHsv({ ...hsv, h: (event.clientX - rect.left) / rect.width * 359 });
  }

  function startLocalDrag(event: PointerEvent, picker: (next: PointerEvent, element: HTMLElement) => void): void {
    if (event.button !== 0) return;
    event.preventDefault();
    const element = event.currentTarget as HTMLElement;
    element.focus({ preventScroll: true });
    picker(event, element);
    api.ui.drag(event, { move: (_dx: number, _dy: number, next: PointerEvent) => picker(next, element), up: () => {} });
  }

  function nudgeSv(event: KeyboardEvent): void {
    if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(event.key)) return;
    event.preventDefault();
    const step = event.shiftKey ? 10 : 1;
    fromHsv({
      h: hsv.h,
      s: hsv.s + (event.key === 'ArrowRight' ? step : event.key === 'ArrowLeft' ? -step : 0),
      v: hsv.v + (event.key === 'ArrowUp' ? step : event.key === 'ArrowDown' ? -step : 0)
    });
  }

  function nudgeHue(event: KeyboardEvent): void {
    if (!['ArrowLeft', 'ArrowRight'].includes(event.key)) return;
    event.preventDefault();
    const step = event.shiftKey ? 10 : 1;
    fromHsv({ ...hsv, h: hsv.h + (event.key === 'ArrowRight' ? step : -step) });
  }

  function show(): void {
    window.clearTimeout(closeTimer);
    previewing = false;
    invalid = false;
    before = normalizeHex(value) ?? '#808080';
    setHex(before, false);
    phase = 'open';
    open = true;
    void tick().then(() => {
      valueInput?.focus();
      valueInput?.select();
    });
  }

  function reducedMotion(): boolean {
    return typeof matchMedia !== 'function' || matchMedia('(prefers-reduced-motion: reduce)').matches;
  }

  /* Play the menu's exit, then unmount. A second close while leaving is a no-op. */
  function finishClose(): void {
    if (!open || phase === 'closed') return;
    phase = 'closed';
    const done = (): void => {
      if (!open) return;
      window.clearTimeout(closeTimer);
      open = false;
      void tick().then(() => trigger?.focus());
    };
    if (reducedMotion()) { done(); return; }
    popover?.addEventListener('animationend', done, { once: true });
    closeTimer = window.setTimeout(done, 200);
  }

  function cancelPreview(): void {
    if (previewing) {
      if (edit.mode === 'local') gesture.write(before);
      gesture.cancel();
      previewing = false;
      api.transport.invalidate('render');
    }
    finishClose();
  }

  function commitAndClose(): void {
    if (phase === 'closed') return;
    const parsed = parse(draft, format);
    if (parsed && parsed !== chosen) commitHex(parsed);
    if (previewing) {
      gesture.write(chosen);
      gesture.commit();
      previewing = false;
      api.transport.invalidate('render');
    } else if (chosen !== before) gesture.once(chosen);
    finishClose();
  }

  function keydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === 'Escape') { event.preventDefault(); cancelPreview(); }
    if (event.key === 'Enter') {
      event.preventDefault();
      if (!parse(draft, format)) { invalid = true; return; }
      commitAndClose();
    }
    if (event.key !== 'Tab' || !popover) return;
    const focusable = [...popover.querySelectorAll<HTMLElement>('button:not([disabled]):not([tabindex="-1"]),input:not([disabled]),[tabindex="0"]')];
    const first = focusable[0], last = focusable.at(-1);
    if (!first || !last) return;
    if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
    else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
  }
</script>

<button
  bind:this={trigger}
  type="button"
  class="color-field"
  aria-haspopup="dialog"
  aria-expanded={open}
  aria-labelledby={labelledBy}
  aria-label={labelledBy ? undefined : `${label} · ${value}`}
  onpointerdown={(event) => event.stopPropagation()}
  onclick={show}
>
  <span style="font-family:var(--f-mono);font-size:var(--fs-md);color:var(--tx)">{isMixed ? 'Mixed' : value.toUpperCase()}</span>
  <span class="sw" aria-hidden="true" style={`--sw-color:${value}`}></span>
</button>

{#if open}
  <div
    class="color-picker-layer"
    role="presentation"
    data-state={phase}
    use:mountOverlayOnBody
    onpointerdown={(event) => { if (event.target === event.currentTarget) commitAndClose(); }}
  >
    <div
      bind:this={popover}
      class="color-picker"
      role="dialog"
      aria-label={label}
      tabindex="-1"
      data-state={phase}
      style={`--cp-color:${chosen};--cp-hue:${hueColor}`}
      use:anchorPicker={trigger}
      onkeydown={keydown}
    >
      <div
        class="color-sv"
        role="slider"
        tabindex="0"
        aria-label="Saturation and brightness"
        aria-valuemin="0"
        aria-valuemax="100"
        aria-valuenow={hsv.s}
        aria-valuetext={`${hsv.s}% saturation, ${hsv.v}% brightness`}
        onpointerdown={(event) => startLocalDrag(event, pickSv)}
        onkeydown={nudgeSv}
      ><i aria-hidden="true" style:left={`${hsv.s}%`} style:top={`${100 - hsv.v}%`}></i></div>

      <div
        class="cp-hue"
        role="slider"
        tabindex="0"
        aria-label="Hue"
        aria-valuemin="0"
        aria-valuemax="359"
        aria-valuenow={hsv.h}
        aria-valuetext={`${hsv.h} degrees`}
        onpointerdown={(event) => startLocalDrag(event, pickHue)}
        onkeydown={nudgeHue}
      ><span class="cp-hue-track"><i aria-hidden="true" style:left={`${hsv.h / 359 * 100}%`}></i></span></div>

      <div class="cp-value">
        <!-- svelte-ignore a11y_interactive_supports_focus (the checked radio is the tab stop) -->
        <div class="cp-formats" role="radiogroup" aria-label="Color format" onkeydown={formatKey}>
          <span class="cp-formats-pill" aria-hidden="true" style:transform={`translateX(${formatIndex * 100}%)`}></span>
          {#each formats as [id, text] (id)}
            <button
              type="button"
              class="cp-format"
              role="radio"
              aria-checked={format === id}
              tabindex={format === id ? 0 : -1}
              onclick={() => switchFormat(id)}
            >{text}</button>
          {/each}
        </div>
        <input
          bind:this={valueInput}
          bind:value={draft}
          class="color-hex"
          aria-label={inputLabel}
          aria-invalid={invalid || undefined}
          spellcheck="false"
          autocomplete="off"
          oninput={typeDraft}
          onblur={settleDraft}
        />
        <button
          type="button"
          class="cp-eyedropper"
          aria-label="Sample screen color"
          title="Sample color from screen"
          aria-busy={sampling}
          disabled={sampling}
          onclick={sampleScreenColor}
        >
          <svg viewBox="0 0 24 24" aria-hidden="true" focusable="false">
            <path d="m15 6 3.4-3.4a2.1 2.1 0 0 1 3 3L18 9m-3-3-6.5 6.5 3 3L18 9m-6.5 6.5L6 21H3v-3l5.5-5.5" />
          </svg>
        </button>
      </div>

      <div class="cp-swatches">
        <div class="cp-compare" aria-label="Original and new color">
          <button
            type="button"
            title={`Original ${before} — click to restore`}
            aria-label={`Restore original color ${before}`}
            style={`--sw-color:${before}`}
            onclick={() => setHex(before)}
          ></button>
          <span title={`New ${chosen}`} style={`--sw-color:${chosen}`}></span>
        </div>
        <div class="cp-presets" role="group" aria-label="Color presets">
          {#each presets as color (color)}
            <button
              type="button"
              class="cp-preset"
              aria-label={color}
              aria-pressed={chosen === color}
              title={color}
              style={`--sw-color:${color}`}
              onclick={() => setHex(color)}
            ></button>
          {/each}
        </div>
      </div>
    </div>
  </div>
{/if}
