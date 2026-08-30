<script lang="ts">
  import { tick } from 'svelte';
  import { doc } from '../state/document.svelte';
  import { transport } from '../state/transport.svelte';
  import { EditGesture, type EditBinding } from './gesture';
  import { rowLabelId } from './context';
  import { clamp, hexToRgb, hsvToRgb, normalizeHex, rgbToHex, rgbToHsv } from './control-utils';
  import './controls.css';

  let {
    PM,
    get,
    edit,
    label = 'Color'
  }: {
    PM: Record<string, any>;
    get: () => unknown;
    edit: EditBinding;
    label?: string;
  } = $props();

  const labelledBy = rowLabelId();
  const raw = $derived((doc.tick.values, doc.proj, transport.time, get()));
  const value = $derived(typeof raw === 'string' && /^#[0-9a-f]{3,8}$/i.test(raw) ? raw : '#808080');
  const gesture = $derived(new EditGesture(PM, edit));
  const presets = ['#09090A', '#FFFFFF', '#FF6B1A', '#FFB000', '#34C759', '#0A84FF', '#6E5AE6', '#FF375F'];
  const hsvChannels = [['h', 'H', '°', 359], ['s', 'S', '%', 100], ['v', 'B', '%', 100]] as const;
  const rgbChannels = [['r', 'R'], ['g', 'G'], ['b', 'B']] as const;

  let trigger = $state<HTMLButtonElement>();
  let dialog = $state<HTMLDivElement>();
  let hexInput = $state<HTMLInputElement>();
  let open = $state(false);
  let pickerLeft = $state(12);
  let pickerTop = $state(52);
  /* `chosen` stays authoritative so typed hex/RGB never drifts through an HSV round trip. */
  let chosen = $state('#808080');
  let before = $state('#808080');
  let hsv = $state({ h: 0, s: 0, v: 50 });
  let draft = $state('#808080');
  let previewing = $state(false);

  const rgb = $derived(hexToRgb(chosen) ?? { r: 0, g: 0, b: 0 });
  const hueColor = $derived(rgbToHex(hsvToRgb({ h: hsv.h, s: 100, v: 100 })));

  /* Typing only re-syncs the wheel; it never rewrites the field mid-keystroke. */
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
    PM.invalidate?.('render');
  }

  function setHex(next: unknown, preview = true): boolean {
    if (!commitHex(next)) return false;
    draft = chosen;
    if (preview) previewChosen();
    return true;
  }

  function typeHex(): void {
    if (/^#?[0-9a-f]{6}$/i.test(draft.trim()) && commitHex(draft)) previewChosen();
  }

  function fromHsv(next: { h: unknown; s: unknown; v: unknown }): void {
    hsv = {
      h: Math.round(clamp(Number(next.h) || 0, 0, 359)),
      s: Math.round(clamp(Number(next.s) || 0, 0, 100)),
      v: Math.round(clamp(Number(next.v) || 0, 0, 100))
    };
    chosen = rgbToHex(hsvToRgb(hsv));
    draft = chosen;
    previewChosen();
  }

  function fromRgb(channel: 'r' | 'g' | 'b', input: HTMLInputElement): void {
    const next = { ...rgb, [channel]: Math.round(clamp(Number(input.value) || 0, 0, 255)) };
    setHex(rgbToHex(next));
    input.value = String(hexToRgb(chosen)![channel]);
  }

  function pickSv(event: PointerEvent, element: HTMLElement): void {
    const rect = element.getBoundingClientRect();
    if (!rect.width || !rect.height) return;
    fromHsv({ h: hsv.h, s: (event.clientX - rect.left) / rect.width * 100, v: 100 - (event.clientY - rect.top) / rect.height * 100 });
  }

  function pickHue(event: PointerEvent, element: HTMLElement): void {
    const rect = element.getBoundingClientRect();
    if (!rect.height) return;
    fromHsv({ ...hsv, h: (event.clientY - rect.top) / rect.height * 359 });
  }

  function startLocalDrag(event: PointerEvent, picker: (next: PointerEvent, element: HTMLElement) => void): void {
    if (event.button !== 0) return;
    event.preventDefault();
    const element = event.currentTarget as HTMLElement;
    picker(event, element);
    PM.drag?.(event, { move: (_dx: number, _dy: number, next: PointerEvent) => picker(next, element), up: () => {} });
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
    if (!['ArrowUp', 'ArrowDown'].includes(event.key)) return;
    event.preventDefault();
    const step = event.shiftKey ? 10 : 1;
    fromHsv({ ...hsv, h: hsv.h + (event.key === 'ArrowDown' ? step : -step) });
  }

  function show(): void {
    previewing = false;
    before = normalizeHex(value) ?? '#808080';
    setHex(before, false);
    const rect = trigger?.getBoundingClientRect();
    const pickerWidth = 384;
    const pickerHeight = 356;
    pickerLeft = clamp((rect?.right ?? pickerWidth + 12) - pickerWidth, 12, window.innerWidth - pickerWidth - 12);
    pickerTop = clamp((rect?.bottom ?? 46) + 6, 52, window.innerHeight - pickerHeight - 12);
    open = true;
    void tick().then(() => { hexInput?.focus(); hexInput?.select(); });
  }

  function finishClose(): void {
    open = false;
    void tick().then(() => trigger?.focus());
  }

  function cancelPreview(): void {
    if (previewing) {
      if (edit.mode === 'local') gesture.write(before);
      gesture.cancel();
      previewing = false;
      PM.invalidate?.('render');
    }
    finishClose();
  }

  function apply(): void {
    if (!commitHex(draft)) { PM.toast?.('Enter a six-digit hex color'); return; }
    if (previewing) {
      gesture.write(chosen);
      gesture.commit();
      previewing = false;
      PM.invalidate?.('render');
    } else gesture.once(chosen);
    finishClose();
  }

  function keydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === 'Escape') { event.preventDefault(); cancelPreview(); }
    if (event.key === 'Enter') { event.preventDefault(); apply(); }
    if (event.key !== 'Tab' || !dialog) return;
    const focusable = [...dialog.querySelectorAll<HTMLElement>('button:not([disabled]),input:not([disabled]),[tabindex="0"]')];
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
  <span style="font-family:var(--f-mono);font-size:var(--fs-md);color:var(--tx)">{value.toUpperCase()}</span>
  <span class="sw" aria-hidden="true" style={`--sw-color:${value}`}></span>
</button>

{#if open}
  <div class="fill-picker-layer" role="presentation" onpointerdown={(event) => { if (event.target === event.currentTarget) cancelPreview(); }}>
    <div bind:this={dialog} class="fill-picker color-picker" role="dialog" aria-modal="true" aria-label={label} tabindex="-1" style:left={`${pickerLeft}px`} style:top={`${pickerTop}px`} onkeydown={keydown}>
      <header><b>{label}</b><button type="button" class="iconbtn" aria-label="Close color picker" onclick={cancelPreview}>×</button></header>
      <div class="fill-picker-body color-dialog">
        <div class="color-workbench">
          <div
            class="fill-sv color-sv"
            role="slider"
            tabindex="0"
            aria-label="Saturation and brightness"
            aria-valuemin="0"
            aria-valuemax="100"
            aria-valuenow={hsv.s}
            aria-valuetext={`${hsv.s}% saturation, ${hsv.v}% brightness`}
            style={`--hue-color:${hueColor}`}
            onpointerdown={(event) => startLocalDrag(event, pickSv)}
            onkeydown={nudgeSv}
          ><i aria-hidden="true" style:left={`${hsv.s}%`} style:top={`${100 - hsv.v}%`}></i></div>

          <div
            class="fill-hue color-hue"
            role="slider"
            tabindex="0"
            aria-label="Hue"
            aria-valuemin="0"
            aria-valuemax="359"
            aria-valuenow={hsv.h}
            onpointerdown={(event) => startLocalDrag(event, pickHue)}
            onkeydown={nudgeHue}
          ><i aria-hidden="true" style:top={`${hsv.h / 359 * 100}%`}></i></div>

          <div class="color-side">
            <div class="color-compare" aria-label="New and original color">
              <span class="color-compare-swatch" title={`New ${chosen}`} style={`--sw-color:${chosen}`}></span>
              <button
                type="button"
                class="color-compare-swatch is-before"
                title={`Original ${before} — click to restore`}
                aria-label={`Restore original color ${before}`}
                style={`--sw-color:${before}`}
                onclick={() => setHex(before)}
              ></button>
            </div>

            <div class="color-channels">
              {#each hsvChannels as [key, text, unit, max]}
                <label class="color-channel">
                  <span>{text}</span>
                  <input
                    class="pm-control-input"
                    inputmode="numeric"
                    aria-label={`${text} ${unit === '°' ? 'degrees' : 'percent'}`}
                    value={hsv[key]}
                    onchange={(event) => fromHsv({ ...hsv, [key]: clamp(Number(event.currentTarget.value) || 0, 0, max) })}
                  />
                  <em>{unit}</em>
                </label>
              {/each}
              <hr aria-hidden="true" />
              {#each rgbChannels as [key, text]}
                <label class="color-channel">
                  <span>{text}</span>
                  <input
                    class="pm-control-input"
                    inputmode="numeric"
                    aria-label={text}
                    value={rgb[key]}
                    onchange={(event) => fromRgb(key, event.currentTarget)}
                  />
                  <em></em>
                </label>
              {/each}
            </div>
          </div>
        </div>

        <div class="color-dialog-value">
          <input
            bind:this={hexInput}
            bind:value={draft}
            class="color-hex pm-control-input"
            aria-label={`${label} hex value`}
            spellcheck="false"
            oninput={typeHex}
          />
        </div>

        <div class="color-grid" role="group" aria-label="Color presets">
          {#each presets as color}
            <button
              type="button"
              class="color-choice"
              class:on={chosen === color}
              aria-label={color}
              title={color}
              style={`--sw-color:${color}`}
              onclick={() => setHex(color)}
            ></button>
          {/each}
        </div>
      </div>
      <footer><button type="button" class="btn" onclick={cancelPreview}>Cancel</button><button type="button" class="btn pri" onclick={apply}>OK</button></footer>
    </div>
  </div>
{/if}
