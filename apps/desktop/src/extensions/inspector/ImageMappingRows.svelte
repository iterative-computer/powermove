<script lang="ts">
  /* How an image fill maps into its frame, as in Figma: Fill, Fit, Crop or
     Tile. Tiles take a scale and rotation; a crop keeps its imported frame. */
  import AnimatedRow from './AnimatedRow.svelte';
  import Choice from './Choice.svelte';
  import { inspectorContext, type EditBinding } from './context';
  import { cloneRaw, contentCommand, IMAGE_MODES, propertyCommand, rawAt } from './figma-data';
  import { perLayerEdit } from './multi-edit';

  let { layer, content }: { layer: any; content: Record<string, any> } = $props();
  const { api, transport, mixed } = inspectorContext();
  const { NumField } = api.ui.controls;

  const mapping = $derived((content.imageMapping ?? {}) as Record<string, any>);
  const mode = $derived(String(mapping.mode ?? 'fill'));
  const has = (key: string) => (void content, rawAt(layer, `imageMapping.${key}`) !== undefined);
  const live = () => api.model.layer(layer.id) ?? layer;

  function field(key: 'mode' | 'scale' | 'rotation', label: string, map: (value: unknown) => unknown = (value) => value): EditBinding {
    return {
      mode: 'command', label, origin: 'inspector',
      command: (value) => perLayerEdit(api, live(), (target) => rawAt(target, `imageMapping.${key}`) !== undefined, (target, exists) => {
        if (target.type !== 'image' || rawAt(target, 'imageMapping') == null) return null;
        if (exists) return propertyCommand(target.id, `c.imageMapping.${key}`, map(value), transport.time);
        const raw = cloneRaw(rawAt(target, 'imageMapping')) as Record<string, unknown>;
        return contentCommand(target.id, { imageMapping: { mode: 'fill', scale: 1, rotation: 0, ...raw, [key]: map(value) } });
      })
    };
  }
  const modeEdit = field('mode', 'Image mode');
  const scaleEdit = field('scale', 'Tile scale', (value) => Math.max(0.01, (Number(value) || 0) / 100));
  const rotationEdit = field('rotation', 'Tile rotation', (value) => Number(value) || 0);
</script>

<AnimatedRow {layer} label="Mode" path={has('mode') ? 'c.imageMapping.mode' : undefined}>
  <Choice value={mode} options={IMAGE_MODES} label="Image mode" edit={modeEdit} />
</AnimatedRow>
{#if mode === 'tile'}
  <AnimatedRow {layer} label="Tile scale" path={has('scale') ? 'c.imageMapping.scale' : undefined}>
    <NumField {api} {mixed} get={() => Math.round(Number(mapping.scale ?? 1) * 1000) / 10} edit={scaleEdit} label="Tile scale" step={1} min={1} unit="%" precision={1} />
  </AnimatedRow>
  <AnimatedRow {layer} label="Rotation" path={has('rotation') ? 'c.imageMapping.rotation' : undefined}>
    <NumField {api} {mixed} get={() => Number(mapping.rotation ?? 0)} edit={rotationEdit} label="Tile rotation" step={1} unit="°" precision={1} />
  </AnimatedRow>
{/if}
