<script lang="ts">
  /* Styled ranges of one text layer, listed compactly: a swatch, the words
     and their style. Opening a style edits just that range, with keyframes
     on its own channels (c.runs.N.*). A list rather than per-selection
     editing keeps each range's channel paths stable: splitting or merging
     ranges from a canvas selection would move their keyframes. */
  import AnimatedRow from './AnimatedRow.svelte';
  import Icon from './Icon.svelte';
  import { inspectorContext, type EditBinding } from './context';
  import { cloneRaw, contentCommand, DECORATIONS, propertyCommand, rawAt, styleName, TEXT_CASES } from './figma-data';

  let { layer, content, fontsVersion = 0 }: { layer: any; content: Record<string, any>; fontsVersion?: number } = $props();
  const { api, transport, mixed } = inspectorContext();
  const { ColorField, FontField, NumField, SelectField } = api.ui.controls;

  const runs = $derived((Array.isArray(content.runs) ? content.runs : []) as Array<Record<string, any>>);
  const text = $derived(String(content.text ?? ''));
  let open = $state<number | null>(null);
  const base = (key: string) => content[key];
  const value = (run: Record<string, any>, key: string) => run[key] ?? base(key);
  const has = (index: number, key: string) => (void content, rawAt(layer, `runs.${index}.${key}`) !== undefined);
  const live = () => api.model.layer(layer.id) ?? layer;

  const excerpt = (run: Record<string, any>) => {
    const words = text.slice(Number(run.start) || 0, Number(run.end) || 0).replace(/\s+/g, ' ').trim();
    return words || '(space)';
  };
  const summary = (run: Record<string, any>) =>
    `${value(run, 'font')} · ${styleName(Number(value(run, 'weight')) || 400, !!value(run, 'italic'))} · ${Math.round((Number(value(run, 'size')) || 0) * 10) / 10}px`;

  function field(index: number, key: string, label: string, map: (value: unknown) => unknown = (value) => value): EditBinding {
    return {
      mode: 'command', label, origin: 'inspector',
      command: (input) => {
        const next = map(input);
        if (rawAt(live(), `runs.${index}.${key}`) !== undefined) return propertyCommand(layer.id, `c.runs.${index}.${key}`, next, transport.time);
        const raw = cloneRaw(rawAt(live(), 'runs')) as Array<Record<string, unknown>>;
        if (!raw?.[index]) return [];
        raw[index]![key] = next;
        return contentCommand(layer.id, { runs: raw });
      }
    };
  }
  /* Style is weight and slope together, as the layer's Style menu. */
  const STYLES = [100, 200, 300, 400, 500, 600, 700, 800, 900].flatMap((weight) => [false, true].map((italic) => ({ v: `${weight}${italic ? 'i' : ''}`, label: styleName(weight, italic) })));
  function styleEdit(index: number): EditBinding {
    const weight = field(index, 'weight', 'Style'), italic = field(index, 'italic', 'Style');
    return {
      mode: 'command', label: 'Style', origin: 'inspector',
      command: (input) => {
        const key = String(input), nextWeight = parseInt(key, 10) || 400, nextItalic = key.endsWith('i');
        api.media.fonts?.ensure?.(String(value(runs[index] ?? {}, 'font')), nextWeight);
        return [weight, italic].flatMap((binding, i) => binding.mode === 'command' && typeof binding.command === 'function'
          ? ([] as unknown[]).concat(binding.command(i ? nextItalic : nextWeight)) : []) as never;
      }
    };
  }
  const styleKey = (run: Record<string, any>) => `${Math.round((Number(value(run, 'weight')) || 400) / 100) * 100}${value(run, 'italic') ? 'i' : ''}`;
</script>

<div class="runs" role="list" aria-label="Text styles" data-text-runs>
  <div class="runs-head"><span>Styles</span><span class="runs-count">{runs.length}</span></div>
  {#each runs as run, index (index)}
    {@const expanded = open === index}
    <div class="run" role="listitem" data-run-index={index}>
      <button type="button" class="run-head" class:open={expanded} aria-expanded={expanded} aria-controls={`run-${layer.id}-${index}`}
        aria-label={`Style ${index + 1}: “${excerpt(run)}”, ${summary(run)}`} onclick={() => { open = expanded ? null : index; }}>
        <span class="twirl" aria-hidden="true"><Icon name={expanded ? 'chevD' : 'chev'} /></span>
        <span class="run-swatch cp-checker" aria-hidden="true"><i style:background={String(value(run, 'color'))}></i></span>
        <span class="run-text">“{excerpt(run)}”</span>
        <span class="run-style">{summary(run)}</span>
      </button>
      {#if expanded}
        <div class="run-rows" id={`run-${layer.id}-${index}`}>
          <AnimatedRow {layer} label="Font" path={has(index, 'font') ? `c.runs.${index}.font` : undefined}>
            {#key fontsVersion}
              <FontField {api} {mixed} get={() => value(run, 'font') ?? ''} edit={field(index, 'font', 'Font')} label="Font"
                weight={() => Number(value(run, 'weight')) || 400} styleName={() => styleName(Number(value(run, 'weight')) || 400, !!value(run, 'italic'))} />
            {/key}
          </AnimatedRow>
          <AnimatedRow {layer} label="Style" path={has(index, 'weight') ? `c.runs.${index}.weight` : undefined}>
            <SelectField {api} {mixed} get={() => styleKey(run)} edit={styleEdit(index)} options={STYLES} label="Style" />
          </AnimatedRow>
          <AnimatedRow {layer} label="Size" path={has(index, 'size') ? `c.runs.${index}.size` : undefined}>
            <NumField {api} {mixed} get={() => value(run, 'size') ?? 0} edit={field(index, 'size', 'Size')} label="Size" step={1} min={1} unit="px" />
          </AnimatedRow>
          <AnimatedRow {layer} label="Color" path={has(index, 'color') ? `c.runs.${index}.color` : undefined}>
            <ColorField {api} {mixed} get={() => value(run, 'color') ?? '#FFFFFF'} edit={field(index, 'color', 'Text color')} label="Text color" />
          </AnimatedRow>
          <AnimatedRow {layer} label="Tracking" path={has(index, 'tracking') ? `c.runs.${index}.tracking` : undefined}>
            <NumField {api} {mixed} get={() => value(run, 'tracking') ?? 0} edit={field(index, 'tracking', 'Tracking')} label="Tracking" step={0.5} unit="px" />
          </AnimatedRow>
          <AnimatedRow {layer} label="Line height" path={has(index, 'leading') ? `c.runs.${index}.leading` : undefined}>
            <NumField {api} {mixed} get={() => Math.round((Number(run.leading) || (Number(content.leading) || 1.2) * (Number(value(run, 'size')) || 0)) * 10) / 10}
              edit={field(index, 'leading', 'Line height', (input) => Math.max(1, Number(input) || 1))} label="Line height" step={1} min={1} unit="px" />
          </AnimatedRow>
          <AnimatedRow {layer} label="Decoration" path={has(index, 'decoration') ? `c.runs.${index}.decoration` : undefined}>
            <SelectField {api} {mixed} get={() => String(value(run, 'decoration') ?? 'NONE')} edit={field(index, 'decoration', 'Decoration')} options={DECORATIONS} label="Decoration" />
          </AnimatedRow>
          <AnimatedRow {layer} label="Case" path={has(index, 'textCase') ? `c.runs.${index}.textCase` : undefined}>
            <SelectField {api} {mixed} get={() => String(value(run, 'textCase') ?? 'ORIGINAL')} edit={field(index, 'textCase', 'Case')} options={TEXT_CASES} label="Case" />
          </AnimatedRow>
        </div>
      {/if}
    </div>
  {/each}
</div>

<style>
  .runs { display: flex; flex-direction: column; gap: 2px; margin-top: 6px; padding: 0 4px; }
  .runs-head { display: flex; align-items: center; justify-content: space-between; height: 24px; color: var(--tx-2); font-size: var(--fs-sm); }
  .runs-count { color: var(--tx-3); font-variant-numeric: tabular-nums; }
  .run-head {
    width: 100%; height: 28px; display: flex; align-items: center; gap: 6px; padding: 0 8px 0 2px;
    border: 0; border-radius: var(--r-sm); background: var(--bg-row); color: var(--tx); font: inherit; font-size: var(--fs-md);
    text-align: left; cursor: pointer;
  }
  .run-head:hover { background: rgb(var(--ink-rgb) / .08); }
  .run-head:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
  .twirl { width: 16px; display: inline-flex; justify-content: center; color: var(--tx-3); flex: none; }
  .twirl :global(svg) { width: 12px; height: 12px; }
  .run-swatch { position: relative; width: 14px; height: 14px; flex: none; border-radius: 4px; overflow: hidden; }
  .run-swatch i { position: absolute; inset: 0; box-shadow: inset 0 0 0 1px rgb(var(--ink-rgb) / .12); border-radius: inherit; }
  .run-text { flex: 0 1 auto; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .run-style { flex: 1 1 0; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--tx-3); text-align: right; font-size: var(--fs-sm); }
  .run-rows { padding: 2px 0 6px; }
</style>
