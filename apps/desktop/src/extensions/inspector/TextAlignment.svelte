<script lang="ts">
  import { inspectorContext } from './context';

  let { value = 'center', layer }: { value?: string; layer: any } = $props();
  const { api, doc, sel, transport, edit: inspectorEdit, mixed: isMixed } = inspectorContext();
  const binding = $derived(api.ui.controls.binding.contentBinding(layer.id, 'align', { label: 'Text alignment', origin: 'inspector' }));
  const mixed = $derived((sel.layers, doc.tick.values, doc.proj, transport.time, isMixed(binding, value)));
  const options = ['left', 'center', 'right'] as const;

  function align(next: string) {
    if (!mixed && value === next) return;
    if (binding.mode === 'command') {
      binding.prepare?.();
      const command = typeof binding.command === 'function' ? binding.command(next) : { ...binding.command, value: next };
      inspectorEdit.apply(command, { label: binding.label, origin: binding.origin });
      api.transport.invalidate?.();
    }
  }
</script>

<div class="text-alignment" role="group" aria-label="Text alignment">
  {#each options as option}
    <button type="button" aria-label={`Align ${option}`} title={`Align ${option}`} aria-pressed={!mixed && value === option} onclick={() => align(option)} onpointerdown={(event) => event.stopPropagation()}>
      <span class="align-glyph" aria-hidden="true">{@html api.ui.icon(option === 'left' ? 'textLeft' : option === 'right' ? 'textRight' : 'textCenter')}</span>
    </button>
  {/each}
</div>

<style>
  .text-alignment { display:flex; gap:2px; width:100%; padding:2px; background:var(--bg-row); border-radius:var(--r-sm); }
  button { display:flex; align-items:center; justify-content:center; flex:1; height:24px; padding:0; border:0; border-radius:calc(var(--r-sm) - 2px); background:transparent; color:var(--tx-3);  }
  button:hover { background:var(--ink-1); color:var(--tx); }
  button:active { background:var(--ink-2); color:var(--tx); transform:scale(.94); transition:none; }
  button[aria-pressed="true"] { background:var(--bg-panel); color:var(--tx); box-shadow:none; }
  button:focus-visible { outline:2px solid var(--accent); outline-offset:-2px; }
</style>
