<script lang="ts">
  /* Render Quality: how hard Studio works on each 3D frame of this
     composition, for Refine in the viewer and for export. Simple by default —
     one preset with a plain description and a time estimate. Custom reveals
     the samples, noise threshold, time per frame, bounces, caustics, denoise
     and 3D motion blur samples. Saved with the composition, one Undo per change. */
  import { onDestroy, untrack } from 'svelte';
  import { inspectorContext } from './context';

  const { api, doc } = inspectorContext();
  const { NumField, Row, Section, SelectField, ToggleField } = api.ui.controls;
  const quality = untrack(() => api.scene3d.renderQuality);

  let current = $state(untrack(() => quality.get()));
  const subscription = untrack(() => quality.onChange(() => { current = quality.get(); }));
  onDestroy(() => subscription.dispose());
  // Undo/redo and composition switches also arrive as document changes.
  $effect(() => { doc.tick.values; doc.tick.structure; doc.proj; current = quality.get(); });

  const about = $derived(quality.describe(current));
  const choices = quality.choices.map(choice => ({ v: choice.id, label: choice.label }));
  const set = (input: Parameters<typeof quality.set>[0], label: string) => {
    const result = quality.set(input, { origin: 'inspector', label });
    if (!result.ok) api.ui.toast(result.message, { error: true });
  };
</script>

<Section {api} title="Render Quality" />
<Row {api} label="Quality">
  <SelectField {api} label="Render Quality" get={() => current.preset} options={choices}
    edit={{ mode: 'set', label: 'Render Quality', set: (next: unknown) => set(next as typeof current.preset, 'Render Quality') }} />
</Row>
<div class="quality-about" data-render-quality-about>
  <p>{about.summary}</p>
  <p class="estimate">{about.perFrame}</p>
</div>
{#if current.preset === 'custom'}
  <div class="quality-custom" role="group" aria-label="Custom render quality">
    {#each quality.fields as field (field.key)}
      <div class="quality-field" title={`${field.label} — ${field.help}`}>
        <Row {api} label={field.label}>
          {#if field.kind === 'toggle'}
            <ToggleField {api} label={field.label} get={() => current[field.key]}
              edit={{ mode: 'set', label: field.label, set: (next: unknown) => set({ [field.key]: !!next }, field.label) }} />
          {:else}
            <NumField {api} label={field.label} get={() => current[field.key]} step={field.step} min={field.min} max={field.max} unit={field.unit} precision={field.precision ?? 0}
              edit={{ mode: 'set', label: field.label, set: (next: unknown) => {
                const value = Number(next);
                if (!Number.isFinite(value)) return;
                const clamped = Math.min(field.max ?? value, Math.max(field.min ?? value, field.precision ? value : Math.round(value)));
                // Keep Min ≤ Max by moving the other end with it.
                const patch: Record<string, number> = { [field.key]: clamped };
                if (field.key === 'minSamples' && clamped > current.maxSamples) patch.maxSamples = clamped;
                if (field.key === 'maxSamples' && clamped < current.minSamples) patch.minSamples = clamped;
                set(patch, field.label);
              } }} />
          {/if}
        </Row>
      </div>
    {/each}
  </div>
{/if}

<style>
  .quality-about { margin: -2px 0 6px; padding: 0 0 0 100px; color: var(--tx-3); font-size: 11px; line-height: 15px; }
  .quality-about p { margin: 0; }
  .quality-about .estimate { color: var(--tx-2); margin-top: 2px; }
  .quality-custom { margin-top: 2px; }
  .quality-field { display: contents; }
</style>
