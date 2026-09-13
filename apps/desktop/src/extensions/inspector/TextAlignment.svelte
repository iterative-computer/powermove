<script lang="ts">
  import { inspectorContext } from './context';
  import { inspectorMixed } from './multi-edit';

  let { value = 'center', layer }: { value?: string; layer: any } = $props();
  const { api, doc, sel, transport, edit: inspectorEdit } = inspectorContext();
  const binding = $derived(api.ui.controls.binding.contentBinding(layer.id, 'align', { label: 'Text alignment', origin: 'inspector' }));
  const mixed = $derived((sel.layers, doc.tick.values, doc.proj, transport.time, inspectorMixed(api, binding, value)));
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
      <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" aria-hidden="true">
        <path d="M2 3h12M2 9h12" />
        <path d={option === 'left' ? 'M2 6h7M2 12h7' : option === 'right' ? 'M7 6h7M7 12h7' : 'M4.5 6h7M4.5 12h7'} />
      </svg>
    </button>
  {/each}
</div>

<style>
  .text-alignment { display:flex; gap:2px; width:100%; padding:2px; background:var(--bg-row); border-radius:var(--r-sm); }
  button { display:flex; align-items:center; justify-content:center; flex:1; height:24px; padding:0; border:0; border-radius:calc(var(--r-sm) - 2px); background:transparent; color:var(--tx-3); cursor:pointer; }
  button:hover { background:var(--bg-hover); color:var(--tx); }
  button[aria-pressed="true"] { background:var(--bg-panel); color:var(--tx); box-shadow:0 1px 3px #0002; }
  button:focus-visible { outline:2px solid var(--accent); outline-offset:-2px; }
</style>
