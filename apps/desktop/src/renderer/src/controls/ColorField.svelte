<script lang="ts" module>
  import type { Notation } from './color-space';

  type Space = 'srgb' | 'p3';
  type HueMode = 'closest' | 'chroma' | 'lightness';

  /* Picker preferences outlive one popover, as they do in Paper. */
  const prefs = $state<{ space: Space; notation: Notation; hueMode: HueMode }>({ space: 'srgb', notation: 'hex', hueMode: 'closest' });
</script>

<script lang="ts">
  /* Colour popover, after Paper's picker: an sRGB | Display P3 header, a
     saturation plane and vertical hue track, the previous | new pair, OKLCH,
     HSL and RGB (or OKLab and P3) channel rows with copy buttons, and one value
     field with a notation menu, beside an opacity track. It previews while you
     pick; click outside or Close commits, Escape cancels. Documents store sRGB
     hex (#RRGGBBAA below full opacity), so a Display P3 colour outside sRGB is
     written clipped and says so. */
  import { sel } from '../state/selection.svelte';
  import { tick } from 'svelte';
  import { doc } from '../state/document.svelte';
  import { transport } from '../state/transport.svelte';
  import { EditGesture, type EditBinding } from './gesture';
  import { rowLabelId } from './context';
  import {
    clamp, clipped, cssColor, formatColor, fromHsl, fromHsv, fromOklab, fromOklch, fromOklchKeepingChroma,
    fromOklchKeepingLightness, fromP3, hex, opacityText, outsideSrgb, parseColor, storedHex, toHsl, toHsv, toOklab, toOklch, toP3,
    type Color
  } from './color-space';
  import { anchorPicker, mountOverlayOnBody } from './overlay';
  import { openPopoverMenu, type PopoverMenuHandle } from './popover-menu';
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

  type Kind = 'plane' | 'hue' | 'alpha';
  type EyeDropperConstructor = new () => { open(): Promise<{ sRGBHex: string }> };

  const hueModes: Array<[HueMode, string]> = [['closest', 'Find closest color'], ['chroma', 'Maintain chroma'], ['lightness', 'Maintain lightness']];
  const notationNames: Record<Notation, string> = { oklch: 'OKLCH', hsl: 'HSL', rgb: 'RGB', hex: 'Hex', 'display-p3': 'Display P3' };
  const grey: Color = { r: 0.5, g: 0.5, b: 0.5, a: 1 };

  const labelledBy = rowLabelId();
  const raw = $derived((doc.tick.values, doc.proj, transport.time, get()));
  const value = $derived(typeof raw === 'string' && /^#[0-9a-f]{3,8}$/i.test(raw) ? raw : '#808080');
  const shown = $derived(parseColor(value) ?? grey);
  const isMixed = $derived((sel.layers, doc.tick.values, doc.proj, transport.time, mixed?.(edit, value) ?? false));
  const gesture = $derived(new EditGesture(api, edit));

  let trigger = $state<HTMLButtonElement>();
  let popover = $state<HTMLDivElement>();
  let valueInput = $state<HTMLInputElement>();
  let settingsButton = $state<HTMLButtonElement>();
  let notationButton = $state<HTMLButtonElement>();
  let open = $state(false);
  let phase = $state<'open' | 'closed'>('open');
  /* Channels stay floats so a P3 colour survives a trip through sRGB views. */
  let color = $state<Color>(grey);
  let previous = $state<Color>(grey);
  let hue = $state(0);
  let saturation = $state(0);
  let brightness = $state(0.5);
  let draft = $state('#808080');
  let dirty = false;
  let invalid = $state(false);
  let previewing = $state(false);
  let sampling = $state(false);
  let menu = $state<{ kind: 'settings' | 'notation'; handle: PopoverMenuHandle } | null>(null);
  let closeTimer: number | undefined;

  const hsl = $derived(toHsl(color));
  const oklch = $derived(toOklch(color));
  const oklab = $derived(toOklab(color));
  const p3 = $derived(toP3(color));
  const outside = $derived(outsideSrgb(color));
  const previewCss = $derived(cssColor(color));
  const opaqueCss = $derived(cssColor({ ...color, a: 1 }));
  const pureHue = $derived.by(() => {
    const pure = fromHsv(hue, 1, 1);
    return prefs.space === 'p3' ? `color(display-p3 ${pure.r} ${pure.g} ${pure.b})` : cssColor(pure);
  });
  const oklchRow = $derived({ id: 'oklch', title: 'OKLCH', labels: ['L', 'C', 'H'], values: [(oklch.l * 100).toFixed(1), oklch.c.toFixed(3), oklch.h.toFixed(1)] });
  const rows = $derived(prefs.space === 'p3' ? [
    oklchRow,
    { id: 'oklab', title: 'OKLab', labels: ['L', 'A', 'B'], values: [(oklab.l * 100).toFixed(1), oklab.a.toFixed(3), oklab.b.toFixed(3)] },
    { id: 'p3', title: 'P3', labels: ['R', 'G', 'B'], values: [p3.r, p3.g, p3.b].map((n) => String(Number(n.toFixed(3)))) }
  ] : [
    oklchRow,
    { id: 'hsl', title: 'HSL', labels: ['H', 'S', 'L'], values: [hsl.h.toFixed(1), (hsl.s * 100).toFixed(1), (hsl.l * 100).toFixed(1)] },
    { id: 'rgb', title: 'RGB', labels: ['R', 'G', 'B'], values: [color.r, color.g, color.b].map((n) => String(Math.round(clamp(n) * 255))) }
  ]);
  const notations = $derived<Notation[]>(prefs.space === 'p3' ? ['oklch', 'display-p3'] : ['oklch', 'hsl', 'rgb', 'hex']);
  const inputLabel = $derived(`${label} ${prefs.notation === 'hex' ? 'hex' : notationNames[prefs.notation]} value`);

  function previewChosen(): void {
    if (phase === 'closed') return;
    if (!previewing) {
      gesture.begin();
      previewing = true;
    }
    gesture.write(storedHex(color));
    api.transport.invalidate('render');
  }

  function syncDraft(): void {
    draft = `${formatColor(color, prefs.notation)} / ${opacityText(color.a)}%`;
    dirty = false;
    invalid = false;
  }

  function hsvOf(next: Color): { h: number; s: number; v: number } {
    return toHsv(prefs.space === 'p3' ? toP3(next) : clipped(next));
  }

  function setColor(next: Color, keepHue = false, preview = true): void {
    color = { r: next.r, g: next.g, b: next.b, a: clamp(next.a) };
    const hsv = hsvOf(color);
    if (!keepHue && hsv.s > 0.00001) hue = hsv.h;
    saturation = hsv.s;
    brightness = hsv.v;
    syncDraft();
    if (preview) previewChosen();
  }

  function planeColor(): Color {
    const plane = fromHsv(hue, saturation, brightness, color.a);
    return prefs.space === 'p3' ? fromP3(plane) : plane;
  }

  function setAlpha(alpha: number): void {
    setColor({ ...color, a: clamp(alpha) }, true);
  }

  function parseDraft(text: string): Color | null {
    const parsed = parseColor(text);
    if (parsed) return parsed;
    const bare = text.replace('°', '');
    if (prefs.notation === 'rgb') return parseColor(`rgb(${bare})`);
    if (prefs.notation === 'hsl') return parseColor(`hsl(${bare})`);
    if (prefs.notation === 'oklch') return parseColor(`oklch(${bare})`);
    if (prefs.notation === 'display-p3') return parseColor(`color(display-p3 ${bare})`);
    return null;
  }

  /* The field reads "<colour> / <opacity>%"; a colour that names its own alpha
     (#RRGGBBAA, rgba(), a slash) sets it, and a bare colour keeps the current one. */
  function readDraft(text: string): Color | null {
    const match = text.match(/^(.*?)\s*\/\s*([\d.]+)%\s*$/);
    const body = (match ? match[1]! : text).trim();
    const parsed = parseDraft(body);
    if (!parsed) return null;
    if (match) return { ...parsed, a: clamp(Number(match[2]) / 100) };
    const explicit = /^(?:#?[\da-f]{4}|#?[\da-f]{8}|transparent)$/i.test(body) || /^(?:rgba|hsla)\(/i.test(body) || body.includes('/');
    return explicit ? parsed : { ...parsed, a: color.a };
  }

  /* A pasted value switches the field to its own notation when this space has it. */
  function useDraft(text: string): boolean {
    const parsed = readDraft(text);
    if (!parsed) return false;
    const normalized = text.trim().toLowerCase();
    let inferred: Notation | undefined;
    if (normalized.startsWith('color(display-p3')) inferred = 'display-p3';
    else if (normalized.startsWith('oklch(')) inferred = 'oklch';
    else if (normalized.startsWith('hsl(') || normalized.startsWith('hsla(')) inferred = 'hsl';
    else if (normalized.startsWith('rgb(') || normalized.startsWith('rgba(')) inferred = 'rgb';
    else if (/^#?[\da-f]{3,8}$/.test(normalized)) inferred = 'hex';
    if (inferred) prefs.notation = (prefs.space === 'p3' ? inferred === 'oklch' || inferred === 'display-p3' : inferred !== 'display-p3') ? inferred : 'oklch';
    setColor(parsed);
    return true;
  }

  /* Leaving the field applies a valid draft and restores the value otherwise. */
  function commitDraft(): void {
    if (!dirty || phase === 'closed') return;
    if (!useDraft(draft)) syncDraft();
  }

  function updateChannel(space: string, index: number, text: string): void {
    const number = Number.parseFloat(text);
    if (!Number.isFinite(number) || phase === 'closed') return;
    if (space === 'p3') {
      const next = { ...p3 };
      if (index === 0) next.r = clamp(number);
      if (index === 1) next.g = clamp(number);
      if (index === 2) next.b = clamp(number);
      setColor(fromP3(next));
    } else if (space === 'oklab') {
      const next = { ...oklab };
      if (index === 0) next.l = clamp(number / 100);
      if (index === 1) next.a = number;
      if (index === 2) next.b = number;
      setColor(fromOklab(next.l, next.a, next.b, color.a));
    } else if (space === 'rgb') {
      const next = clipped(color);
      if (index === 0) next.r = clamp(number / 255);
      if (index === 1) next.g = clamp(number / 255);
      if (index === 2) next.b = clamp(number / 255);
      setColor(next);
    } else if (space === 'hsl') {
      const next = { ...hsl };
      if (index === 0) next.h = ((number % 360) + 360) % 360;
      if (index === 1) next.s = clamp(number / 100);
      if (index === 2) next.l = clamp(number / 100);
      setColor(fromHsl(next.h, next.s, next.l, color.a));
    } else {
      const next = { ...oklch };
      if (index === 0) next.l = clamp(number / 100);
      if (index === 1) next.c = clamp(number, 0, 0.4);
      if (index === 2) next.h = ((number % 360) + 360) % 360;
      setColor(fromOklch(next.l, next.c, next.h, color.a));
    }
  }

  function switchSpace(space: Space): void {
    menu?.handle.close();
    prefs.space = space;
    prefs.notation = 'oklch';
    const hsv = hsvOf(color);
    hue = hsv.h;
    saturation = hsv.s;
    brightness = hsv.v;
    syncDraft();
  }

  function changeHue(nextHue: number): void {
    if (prefs.hueMode === 'closest' || saturation === 0) {
      hue = nextHue;
      setColor(planeColor(), true);
      return;
    }
    const old = toOklch(color);
    const shifted = (old.h + nextHue - hue + 360) % 360;
    hue = nextHue;
    setColor(prefs.hueMode === 'lightness' ? fromOklchKeepingLightness(old.l, old.c, shifted, color.a) : fromOklchKeepingChroma(old.l, old.c, shifted, color.a), true);
  }

  function pick(event: PointerEvent, element: HTMLElement, kind: Kind): void {
    const rect = element.getBoundingClientRect();
    if (!rect.width || !rect.height) return;
    const x = clamp((event.clientX - rect.left) / rect.width);
    const y = clamp((event.clientY - rect.top) / rect.height);
    if (kind === 'hue') { changeHue(y * 360); return; }
    if (kind === 'alpha') { setAlpha(1 - y); return; }
    saturation = x;
    brightness = 1 - y;
    setColor(planeColor(), true);
  }

  function startDrag(event: PointerEvent, kind: Kind): void {
    if (event.button !== 0) return;
    event.preventDefault();
    const element = event.currentTarget as HTMLElement;
    element.focus({ preventScroll: true });
    pick(event, element, kind);
    api.ui.drag(event, { move: (_dx: number, _dy: number, next: PointerEvent) => pick(next, element, kind), up: () => {} });
  }

  function sliderKey(event: KeyboardEvent, kind: Kind): void {
    const delta = event.key === 'ArrowUp' || event.key === 'ArrowRight' ? 1 : event.key === 'ArrowDown' || event.key === 'ArrowLeft' ? -1 : 0;
    if (!delta) return;
    event.preventDefault();
    const step = event.shiftKey ? 10 : 1;
    if (kind === 'hue') { changeHue((hue + delta * step + 360) % 360); return; }
    if (kind === 'alpha') { setAlpha(color.a + delta * step / 100); return; }
    if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') saturation = clamp(saturation + delta * step / 100);
    else brightness = clamp(brightness + delta * step / 100);
    setColor(planeColor(), true);
  }

  async function copy(text: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(text);
      api.ui.toast(`Copied ${text}`, { key: 'color-copy' });
    } catch {
      api.ui.toast('Could not copy', { key: 'color-copy', error: true });
    }
  }

  function copyRow(id: string): Promise<void> {
    if (id === 'p3') return copy(`color(display-p3 ${[p3.r, p3.g, p3.b].map((n) => n.toFixed(3)).join(' ')})`);
    if (id === 'oklab') return copy(`oklab(${(oklab.l * 100).toFixed(1)}% ${oklab.a.toFixed(3)} ${oklab.b.toFixed(3)})`);
    if (id === 'rgb') return copy(`rgb(${[color.r, color.g, color.b].map((n) => Math.round(clamp(n) * 255)).join(', ')})`);
    if (id === 'hsl') return copy(`hsl(${Math.round(hsl.h)} ${Math.round(hsl.s * 100)}% ${Math.round(hsl.l * 100)}%)`);
    return copy(`oklch(${(oklch.l * 100).toFixed(1)}% ${oklch.c.toFixed(4)} ${oklch.h.toFixed(2)})`);
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
      useDraft(result.sRGBHex);
    } catch (error) {
      if (!(error instanceof DOMException && error.name === 'AbortError')) api.ui.toast('Could not sample that color');
    } finally {
      sampling = false;
    }
  }

  function toggleMenu(kind: 'settings' | 'notation'): void {
    const wasOpen = menu?.kind === kind;
    menu?.handle.close();
    if (wasOpen) return;
    const anchor = kind === 'settings' ? settingsButton : notationButton;
    if (!anchor) return;
    const settle = (): void => anchor.focus({ preventScroll: true });
    let header: HTMLElement | undefined;
    if (kind === 'settings') {
      header = document.createElement('div');
      header.className = 'cp-menu-heading';
      header.textContent = 'When changing hue…';
    }
    const items = kind === 'settings'
      ? hueModes.map(([id, text]) => ({ label: text, checked: prefs.hueMode === id, run: () => { prefs.hueMode = id; settle(); } }))
      : notations.map((id) => ({ label: notationNames[id], checked: prefs.notation === id, run: () => { prefs.notation = id; syncDraft(); settle(); } }));
    const handle = openPopoverMenu({
      anchor,
      label: kind === 'settings' ? 'Picker settings' : 'Color notation',
      header,
      items,
      side: kind === 'settings' ? 'bottom' : 'top',
      onClose: () => { if (menu?.handle === handle) menu = null; }
    });
    menu = { kind, handle };
  }

  function show(): void {
    window.clearTimeout(closeTimer);
    previewing = false;
    phase = 'open';
    previous = parseColor(value) ?? grey;
    setColor(previous, false, false);
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
    menu?.handle.close();
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
      if (edit.mode === 'local') gesture.write(storedHex(previous));
      gesture.cancel();
      previewing = false;
      api.transport.invalidate('render');
    }
    finishClose();
  }

  function commitAndClose(): void {
    if (phase === 'closed') return;
    // A field still being typed in applies first, as leaving it would.
    const active = document.activeElement;
    if (active instanceof HTMLInputElement && popover?.contains(active)) active.blur();
    commitDraft();
    const chosen = storedHex(color);
    if (previewing) {
      gesture.write(chosen);
      gesture.commit();
      previewing = false;
      api.transport.invalidate('render');
    } else if (chosen !== storedHex(previous)) gesture.once(chosen);
    finishClose();
  }

  function valueKey(event: KeyboardEvent): void {
    if (event.key !== 'Enter') return;
    event.preventDefault();
    if (dirty && !readDraft(draft)) { invalid = true; return; }
    commitAndClose();
  }

  function keydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === 'Escape') { event.preventDefault(); cancelPreview(); }
    // Enter on the plane or a track commits, as it does from the value field.
    if (event.key === 'Enter' && (event.target as HTMLElement).getAttribute('role') === 'slider') { event.preventDefault(); commitAndClose(); }
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
  <span style="font-family:var(--f-mono);font-size:var(--fs-md);color:var(--tx)">{isMixed ? 'Mixed' : hex(shown)}{#if !isMixed && shown.a < 1}<span class="color-field-alpha">{opacityText(shown.a)}%</span>{/if}</span>
  <span class="sw cp-checker" aria-hidden="true" style={`--sw-color:${cssColor(shown)}`}></span>
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
      use:anchorPicker={trigger}
      onkeydown={keydown}
    >
      <div class="cp-header">
        <div class="cp-spaces" role="tablist" aria-label="Color space">
          <button type="button" role="tab" aria-selected={prefs.space === 'srgb'} onclick={() => switchSpace('srgb')}>sRGB</button>
          <button type="button" role="tab" aria-selected={prefs.space === 'p3'} onclick={() => switchSpace('p3')}>Display P3</button>
        </div>
        <div class="cp-actions">
          <button
            type="button"
            class="cp-icon"
            aria-label="Sample screen color"
            title="Sample color from screen"
            aria-busy={sampling}
            disabled={sampling}
            onclick={sampleScreenColor}
          >
            <!-- Phosphor regular: Eyedropper, the app's icon set. -->
            <svg viewBox="0 0 256 256" aria-hidden="true" focusable="false" fill="currentColor">
              <path d="M224,67.3a35.79,35.79,0,0,0-11.26-25.66c-14-13.28-36.72-12.78-50.62,1.13L142.8,62.2a24,24,0,0,0-33.14.77l-9,9a16,16,0,0,0,0,22.64l2,2.06-51,51a39.75,39.75,0,0,0-10.53,38l-8,18.41A13.68,13.68,0,0,0,36,219.3a15.92,15.92,0,0,0,17.71,3.35L71.23,215a39.89,39.89,0,0,0,37.06-10.75l51-51,2.06,2.06a16,16,0,0,0,22.62,0l9-9a24,24,0,0,0,.74-33.18l19.75-19.87A35.75,35.75,0,0,0,224,67.3ZM97,193a24,24,0,0,1-24,6,8,8,0,0,0-5.55.31l-18.1,7.91L57,189.41a8,8,0,0,0,.25-5.75A23.88,23.88,0,0,1,63,159l51-51,33.94,34ZM202.13,82l-25.37,25.52a8,8,0,0,0,0,11.3l4.89,4.89a8,8,0,0,1,0,11.32l-9,9L112,83.26l9-9a8,8,0,0,1,11.31,0l4.89,4.89a8,8,0,0,0,11.33,0l24.94-25.09c7.81-7.82,20.5-8.18,28.29-.81a20,20,0,0,1,.39,28.7Z" />
            </svg>
          </button>
          <button
            bind:this={settingsButton}
            type="button"
            class="cp-icon"
            aria-label="Picker settings"
            title="Picker settings"
            aria-haspopup="menu"
            aria-expanded={menu?.kind === 'settings'}
            onclick={() => toggleMenu('settings')}
          >
            <!-- Phosphor regular: List. -->
            <svg viewBox="0 0 256 256" aria-hidden="true" focusable="false" fill="currentColor">
              <path d="M224,128a8,8,0,0,1-8,8H40a8,8,0,0,1,0-16H216A8,8,0,0,1,224,128ZM40,72H216a8,8,0,0,0,0-16H40a8,8,0,0,0,0,16ZM216,184H40a8,8,0,0,0,0,16H216a8,8,0,0,0,0-16Z" />
            </svg>
          </button>
          <button type="button" class="cp-icon" aria-label="Close" title="Close" onclick={commitAndClose}>
            <!-- Phosphor regular: X. -->
            <svg viewBox="0 0 256 256" aria-hidden="true" focusable="false" fill="currentColor">
              <path d="M205.66,194.34a8,8,0,0,1-11.32,11.32L128,139.31,61.66,205.66a8,8,0,0,1-11.32-11.32L116.69,128,50.34,61.66A8,8,0,0,1,61.66,50.34L128,116.69l66.34-66.35a8,8,0,0,1,11.32,11.32L139.31,128Z" />
            </svg>
          </button>
        </div>
      </div>

      <div class="cp-body">
        <div
          class="cp-plane"
          role="slider"
          tabindex="0"
          aria-label="Saturation and brightness"
          aria-valuemin="0"
          aria-valuemax="100"
          aria-valuenow={Math.round(saturation * 100)}
          aria-valuetext={`${Math.round(saturation * 100)}% saturation, ${Math.round(brightness * 100)}% brightness`}
          style:--cp-hue={pureHue}
          onpointerdown={(event) => startDrag(event, 'plane')}
          onkeydown={(event) => sliderKey(event, 'plane')}
        ><i aria-hidden="true" style:left={`${saturation * 100}%`} style:top={`${(1 - brightness) * 100}%`}></i></div>

        <div
          class="cp-track cp-alpha cp-checker"
          role="slider"
          tabindex="0"
          aria-label="Opacity"
          aria-valuemin="0"
          aria-valuemax="100"
          aria-valuenow={Math.round(color.a * 100)}
          aria-valuetext={`${opacityText(color.a)}%`}
          style:--cp-opaque={opaqueCss}
          onpointerdown={(event) => startDrag(event, 'alpha')}
          onkeydown={(event) => sliderKey(event, 'alpha')}
        ><i aria-hidden="true" style:top={`${(1 - color.a) * 100}%`}></i></div>

        <div
          class="cp-track cp-hue"
          role="slider"
          tabindex="0"
          aria-label="Hue"
          aria-valuemin="0"
          aria-valuemax="360"
          aria-valuenow={Math.round(hue)}
          aria-valuetext={`${Math.round(hue)} degrees`}
          onpointerdown={(event) => startDrag(event, 'hue')}
          onkeydown={(event) => sliderKey(event, 'hue')}
        ><i aria-hidden="true" style:top={`${hue / 360 * 100}%`}></i></div>

        <div class="cp-details">
          <div class="cp-preview cp-checker">
            <button
              type="button"
              class="cp-previous"
              aria-label="Restore the color this picker opened with"
              title="Restore previous color"
              style:background={cssColor(previous)}
              onclick={() => setColor(previous)}
            ></button>
            <span class="cp-new" style:background={previewCss}></span>
            {#if outside}<span class="cp-clipped" title="Outside sRGB; saved as the nearest sRGB color">Clipped</span>{/if}
          </div>
          <div class="cp-preview-labels" aria-hidden="true"><span>Previous</span><span>New</span></div>

          {#each rows as row (row.id)}
            <div class="cp-row" role="group" aria-label={row.title}>
              <div class="cp-channels">
                {#each row.values as channel, index (index)}
                  <div class="cp-channel">
                    <input
                      aria-label={`${row.title} ${row.labels[index]}`}
                      type="number"
                      step="any"
                      value={channel}
                      onchange={(event) => updateChannel(row.id, index, event.currentTarget.value)}
                      onkeydown={(event) => { if (event.key === 'Enter') event.currentTarget.blur(); }}
                    />
                    <span aria-hidden="true">{row.labels[index]}</span>
                  </div>
                {/each}
              </div>
              <button type="button" class="cp-square" aria-label={`Copy ${row.title}`} title={`Copy ${row.title}`} onclick={() => copyRow(row.id)}>
                <!-- Phosphor regular: Copy. -->
                <svg viewBox="0 0 256 256" aria-hidden="true" focusable="false" fill="currentColor">
                  <path d="M216,32H88a8,8,0,0,0-8,8V80H40a8,8,0,0,0-8,8V216a8,8,0,0,0,8,8H168a8,8,0,0,0,8-8V176h40a8,8,0,0,0,8-8V40A8,8,0,0,0,216,32ZM160,208H48V96H160Zm48-48H176V88a8,8,0,0,0-8-8H96V48H208Z" />
                </svg>
              </button>
            </div>
          {/each}

          <div class="cp-value-row">
            <input
              bind:this={valueInput}
              bind:value={draft}
              class="cp-value"
              aria-label={inputLabel}
              aria-invalid={invalid || undefined}
              spellcheck="false"
              autocomplete="off"
              oninput={() => { dirty = true; invalid = false; }}
              onkeydown={valueKey}
              onblur={commitDraft}
            />
            <button
              bind:this={notationButton}
              type="button"
              class="cp-square"
              aria-label="Color notation"
              title="Color notation"
              aria-haspopup="menu"
              aria-expanded={menu?.kind === 'notation'}
              onclick={() => toggleMenu('notation')}
            >
              <!-- Phosphor regular: CaretUpDown. -->
              <svg viewBox="0 0 256 256" aria-hidden="true" focusable="false" fill="currentColor">
                <path d="M181.66,170.34a8,8,0,0,1,0,11.32l-48,48a8,8,0,0,1-11.32,0l-48-48a8,8,0,0,1,11.32-11.32L128,212.69l42.34-42.35A8,8,0,0,1,181.66,170.34Zm-96-84.68L128,43.31l42.34,42.35a8,8,0,0,0,11.32-11.32l-48-48a8,8,0,0,0-11.32,0l-48,48A8,8,0,0,0,85.66,85.66Z" />
              </svg>
            </button>
          </div>
        </div>
      </div>
    </div>
  </div>
{/if}
