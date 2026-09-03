<script lang="ts">
  import ChannelRow from './ChannelRow.svelte';
  import {
    axisMeta, discoverVariableFontAxes, fontAxisChannelPath, fontAxisContentKey,
    normalizeAxisTag, persistedFontAxes, type FontVariationAxis
  } from './variable-font';

  let { PM, layer, content }: { PM: Record<string, any>; layer: any; content: Record<string, any> } = $props();
  let detected = $state<FontVariationAxis[]>([]);
  let customTag = $state('');
  let selectedTag = $state('');
  let request = 0;

  const enabled = $derived(persistedFontAxes(content, detected));
  const enabledTags = $derived(new Set(enabled.map((axis) => axis.tag)));
  const available = $derived(detected.filter((axis) => !enabledTags.has(axis.tag)));

  $effect(() => {
    const family = String(content.font ?? '');
    const token = ++request;
    detected = [];
    selectedTag = '';
    void discoverVariableFontAxes(family).then((axes) => {
      if (token !== request) return;
      detected = axes;
      selectedTag = axes.find((axis) => !enabledTags.has(axis.tag))?.tag ?? '';
    });
  });

  function addAxis(rawTag: string): void {
    const tag = normalizeAxisTag(rawTag);
    if (!tag || enabledTags.has(tag)) return;
    const definition = detected.find((axis) => axis.tag === tag)
      ?? axisMeta(tag, tag === 'wght' ? Number(content.weight) || 400 : 0);
    const result = PM.Edit.apply({
      type: 'set_content',
      target: layer.id,
      patch: { [fontAxisContentKey(tag)]: PM.P(definition.default) }
    }, { label: `Add ${definition.label} axis`, origin: 'inspector' });
    if (result?.ok === false) return;
    customTag = '';
    PM.invalidate?.();
    PM.Inspector?.refresh?.();
  }
</script>

<details class="font-variations" data-font-variations>
  <summary>
    <span>Variable font</span>
    <span class="axis-count">{enabled.length ? `${enabled.length} ${enabled.length === 1 ? 'axis' : 'axes'}` : 'Add axes'}</span>
  </summary>

  <div class="axis-body">
    {#each enabled as axis (axis.tag)}
      {@const property = content[fontAxisContentKey(axis.tag)]}
      <ChannelRow
        {PM}
        {layer}
        channel={fontAxisChannelPath(axis.tag)}
        label={axis.label === axis.tag ? axis.tag : `${axis.label} · ${axis.tag}`}
        {property}
        getValue={(time) => PM.evP(layer, property, time, fontAxisChannelPath(axis.tag))}
        min={axis.min}
        max={axis.max}
        step={axis.tag === 'ital' ? 1 : Math.max(0.01, (axis.max - axis.min) / 200)}
        precision={axis.tag === 'ital' ? 0 : 2}
      />
    {/each}

    {#if available.length}
      <div class="axis-add detected-axis-add">
        <select bind:value={selectedTag} aria-label="Available variable font axis">
          {#each available as axis}<option value={axis.tag}>{axis.label} · {axis.tag}</option>{/each}
        </select>
        <button type="button" class="chip" onclick={() => addAxis(selectedTag)}>Add</button>
      </div>
    {/if}

    <div class="axis-add custom-axis-add">
      <input
        aria-label="Custom OpenType axis tag"
        maxlength="4"
        placeholder="Axis tag"
        value={customTag}
        oninput={(event) => { customTag = event.currentTarget.value; }}
        onkeydown={(event) => {
          event.stopPropagation();
          if (event.key === 'Enter') { event.preventDefault(); addAxis(customTag); }
        }}
      />
      <button type="button" class="chip" disabled={!normalizeAxisTag(customTag)} onclick={() => addAxis(customTag)}>Add</button>
    </div>
    <div class="axis-hint">Use any four-character OpenType axis tag.</div>
  </div>
</details>

<style>
  .font-variations {
    margin-top: 3px;
    border-top: 1px solid var(--line);
  }
  summary {
    display: flex;
    min-height: 30px;
    align-items: center;
    justify-content: space-between;
    padding: 0 4px;
    color: var(--tx-2);
    font-size: var(--fs-xs);
    cursor: default;
    list-style: none;
  }
  summary::-webkit-details-marker { display: none; }
  summary::before {
    width: 12px;
    margin-right: 5px;
    color: var(--tx-4);
    content: '›';
    transform: rotate(0deg);
    transition: transform var(--dur-1);
  }
  details[open] summary::before { transform: rotate(90deg); }
  summary > :first-child { margin-right: auto; }
  .axis-count { color: var(--tx-4); }
  .axis-body { padding: 0 0 5px; }
  .axis-add {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 6px;
    padding: 4px;
  }
  .axis-add select,
  .axis-add input {
    width: 100%;
    min-width: 0;
    height: 28px;
    padding: 0 8px;
    border: 1px solid var(--line);
    border-radius: var(--r-sm);
    background: var(--bg-row);
    color: var(--tx);
    font: inherit;
  }
  .axis-add input { font-family: var(--font-mono); letter-spacing: .06em; }
  .axis-add .chip { min-width: 42px; height: 28px; justify-content: center; }
  .axis-hint { padding: 1px 5px 4px; color: var(--tx-4); font-size: var(--fs-xs); }
</style>
