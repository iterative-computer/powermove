<script lang="ts">
  import { untrack } from 'svelte';
  import { selectionSummary, type PresetChoices, type PresetSelection } from './model';

  /* The body of Save Animation Preset…, after After Effects' Animation ▸
     Save Animation Preset: a name, what to include, whether keyframes come
     along, and one line saying what will be saved. The modal owns the
     buttons and shows the engine's validation errors inline. */
  interface Props {
    choices: PresetChoices;
    initial: PresetSelection;
    name: string;
    layerName: string;
    /** The sheet hands its current state back to the Save action. */
    bridge: { read?: () => { name: string; selection: PresetSelection; includeKeyframes: boolean } };
    /** The app's Off/On control (PM.toggleField), so the sheet matches Pre-compose. */
    toggleField?: (get: () => boolean, set: (value: boolean) => void, options: { local: boolean; label: string }) => HTMLElement;
  }

  let { choices, initial, name: initialName, layerName, bridge, toggleField }: Props = $props();

  // Seeded once from props; afterwards the sheet's own state is the source.
  const seed = untrack(() => ({ choices, initial, initialName }));
  let name = $state(seed.initialName);
  let effectIds = $state<string[]>([...seed.initial.effectIds]);
  let propertyIds = $state<string[]>([...seed.initial.propertyIds]);
  let includeKeyframes = $state(true);
  let showAll = $state(false);
  let nameEl: HTMLInputElement | undefined = $state();

  untrack(() => bridge).read = () => ({ name, selection: { effectIds: [...effectIds], propertyIds: [...propertyIds] }, includeKeyframes });

  /* Unanimated properties stay folded away unless picked, so a long
     Transform/Content list never buries what matters. */
  const hiddenCount = $derived(choices.properties.filter((row) => row.keys === 0 && !initial.propertyIds.includes(row.id)).length);
  const visibleProperties = $derived(showAll || choices.properties.every((row) => row.keys === 0) && !initial.propertyIds.length
    ? choices.properties
    : choices.properties.filter((row) => row.keys > 0 || initial.propertyIds.includes(row.id) || propertyIds.includes(row.id)));
  const anyKeys = $derived([...choices.effects, ...choices.properties].some((item) => item.keys > 0));
  const summary = $derived(selectionSummary(choices, { effectIds, propertyIds }, includeKeyframes));

  function toggle(list: 'effects' | 'properties', id: string): void {
    if (list === 'effects') effectIds = effectIds.includes(id) ? effectIds.filter((other) => other !== id) : [...effectIds, id];
    else propertyIds = propertyIds.includes(id) ? propertyIds.filter((other) => other !== id) : [...propertyIds, id];
  }

  /* Mounts PM.toggleField once; its buttons follow whether anything is animated. */
  function legacyToggle(host: HTMLElement, enabled: boolean) {
    const field = toggleField!(() => includeKeyframes, (value) => { includeKeyframes = value; }, { local: true, label: 'Include keyframes' });
    host.append(field);
    const sync = (on: boolean) => field.querySelectorAll('button').forEach((button) => { button.disabled = !on; });
    sync(enabled);
    return { update: sync, destroy: () => field.remove() };
  }

  const keysLabel = (keys: number) => keys ? `${keys} ${keys === 1 ? 'key' : 'keys'}` : '';

  $effect(() => {
    const input = nameEl;
    if (!input) return;
    const timer = window.setTimeout(() => { input.focus(); input.select(); }, 30);
    return () => window.clearTimeout(timer);
  });
</script>

{#snippet check(on: boolean)}
  <span class="checkbox-field" class:on aria-hidden="true">{#if on}<svg viewBox="0 0 256 256"><path d="M229.66,77.66l-128,128a8,8,0,0,1-11.32,0l-56-56a8,8,0,0,1,11.32-11.32L96,188.69,218.34,66.34a8,8,0,0,1,11.32,11.32Z"/></svg>{/if}</span>
{/snippet}

<div class="new-project-form preset-sheet" data-preset-sheet>
  <label class="new-project-field">
    <span>Name</span>
    <input bind:this={nameEl} class="settings-input is-wide" type="text" maxlength="80" spellcheck="false" aria-label="Preset name" bind:value={name} />
  </label>

  <div class="preset-include" role="group" aria-label={`Include from ${layerName}`}>
    <div class="preset-include-head">
      <span>Include from <b>{layerName}</b></span>
    </div>
    <div class="preset-list">
      {#if choices.effects.length}
        <div class="preset-group" role="presentation">Effects</div>
        {#each choices.effects as effect (effect.id)}
          <button type="button" class="preset-choice" role="checkbox" aria-checked={effectIds.includes(effect.id)} data-preset-effect={effect.id}
            onclick={() => toggle('effects', effect.id)}>
            {@render check(effectIds.includes(effect.id))}
            <span class="preset-badge" aria-hidden="true">fx</span>
            <span class="preset-name">{effect.label}</span>
            <span class="preset-keys">{#if effect.keys}<svg viewBox="0 0 10 10" aria-hidden="true"><path d="M5 1 9 5 5 9 1 5Z"/></svg>{/if}{keysLabel(effect.keys)}</span>
          </button>
        {/each}
      {/if}
      {#if choices.properties.length}
        <div class="preset-group" role="presentation">Properties</div>
        {#each visibleProperties as row (row.id)}
          <button type="button" class="preset-choice" role="checkbox" aria-checked={propertyIds.includes(row.id)} data-preset-property={row.id}
            onclick={() => toggle('properties', row.id)}>
            {@render check(propertyIds.includes(row.id))}
            <span class="preset-name">{row.label}</span>
            {#if row.group !== 'Transform'}<span class="preset-where">{row.group}</span>{/if}
            <span class="preset-keys">{#if row.keys}<svg viewBox="0 0 10 10" aria-hidden="true"><path d="M5 1 9 5 5 9 1 5Z"/></svg>{/if}{keysLabel(row.keys)}</span>
          </button>
        {/each}
        {#if hiddenCount && visibleProperties.length < choices.properties.length}
          <button type="button" class="preset-more" onclick={() => showAll = true}>Show {hiddenCount} more {hiddenCount === 1 ? 'property' : 'properties'}</button>
        {/if}
      {/if}
      {#if !choices.effects.length && !choices.properties.length}
        <p class="preset-empty">{layerName} has no effects or animatable properties to save.</p>
      {/if}
    </div>
  </div>

  <div class="new-project-field" class:is-dim={!anyKeys} title={anyKeys ? '' : 'Nothing here is animated yet, so only values are saved.'}>
    <span>Include keyframes</span>
    {#if toggleField}
      <div class="preset-toggle" use:legacyToggle={anyKeys}></div>
    {:else}
      <button type="button" class="chip" role="switch" aria-checked={includeKeyframes} aria-label="Include keyframes" disabled={!anyKeys} onclick={() => includeKeyframes = !includeKeyframes}>{includeKeyframes ? 'On' : 'Off'}</button>
    {/if}
  </div>

  <p class="precompose-note precompose-summary" aria-live="polite" data-preset-summary>{summary}</p>
</div>

<style>
  .preset-sheet { gap: 10px; }

  .preset-include { display: flex; flex-direction: column; gap: 6px; }
  .preset-include-head { color: var(--tx-2); font-size: var(--fs-sm); }
  .preset-include-head b { color: var(--tx); font-weight: 500; }

  /* A grouped, inset list, as a macOS sheet shows its options. */
  .preset-list {
    display: flex;
    flex-direction: column;
    max-height: 232px;
    overflow: auto;
    padding: 4px;
    border-radius: var(--r-md, 8px);
    background: var(--bg-field);
  }
  .preset-group {
    padding: 6px 8px 2px;
    color: var(--tx-3);
    font-size: var(--fs-xs);
    font-weight: 500;
  }
  .preset-choice {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    min-height: 26px;
    padding: 0 8px;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--tx);
    font: inherit;
    font-size: var(--fs-sm);
    text-align: left;
    cursor: default;
  }
  .preset-choice:hover { background: color-mix(in srgb, var(--tx) 5%, transparent); }
  .preset-choice:focus-visible { outline: 0; box-shadow: var(--focus-ring); }
  .preset-choice[aria-checked='false'] .preset-name { color: var(--tx-2); }
  .preset-choice .checkbox-field { width: 14px; height: 14px; }
  .preset-badge {
    flex: none;
    padding: 0 4px;
    border-radius: 3px;
    background: var(--ink-2);
    color: var(--tx-2);
    font-family: var(--f-mono);
    font-size: 9px;
    line-height: 14px;
  }
  .preset-name { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .preset-where { flex: none; color: var(--tx-4); font-size: var(--fs-xs); }
  .preset-keys {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    flex: none;
    margin-left: auto;
    color: var(--tx-3);
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
  }
  .preset-keys svg { width: 8px; height: 8px; fill: var(--key, var(--accent)); }
  .preset-more {
    align-self: flex-start;
    margin: 2px 0 2px 30px;
    padding: 2px 0;
    border: 0;
    background: none;
    color: var(--accent);
    font: inherit;
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .preset-more:focus-visible { outline: 0; box-shadow: var(--focus-ring); border-radius: 3px; }
  .preset-toggle { display: block; }
  .preset-empty { margin: 0; padding: 10px 8px; color: var(--tx-3); font-size: var(--fs-sm); }

</style>
