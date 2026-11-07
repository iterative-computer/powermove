<script lang="ts">
  /* A few mutually exclusive values as one row of buttons, in the Text
     alignment control's look. Mixed selections press nothing; each button
     stays in the tab order with its own accessible name. */
  import { inspectorContext, type EditBinding } from './context';

  let { value, options, label, edit, mixed: isMixed }: {
    value: unknown;
    options: ReadonlyArray<{ v: string; label: string; title?: string }>;
    label: string;
    edit: EditBinding;
    mixed?: (binding: EditBinding, value: unknown) => boolean;
  } = $props();
  const { api, doc, sel, transport, edit: inspectorEdit, mixed: selectionMixed } = inspectorContext();
  const mixed = $derived((sel.layers, doc.tick.values, doc.proj, transport.time, (isMixed ?? selectionMixed)(edit, value)));

  function choose(next: string): void {
    if (!mixed && value === next) return;
    if (edit.mode !== 'command') return;
    edit.prepare?.();
    const command = typeof edit.command === 'function' ? edit.command(next) : { ...edit.command, value: next };
    inspectorEdit.apply(command, { label: edit.label, origin: edit.origin ?? 'inspector' });
    api.transport.invalidate?.();
  }
</script>

<div class="choice" role="group" aria-label={label} data-choice={label}>
  {#each options as option (option.v)}
    <button type="button" aria-pressed={!mixed && value === option.v} title={option.title ?? option.label}
      onclick={() => choose(option.v)} onpointerdown={(event) => event.stopPropagation()}>{option.label}</button>
  {/each}
</div>

<style>
  .choice { display:flex; gap:2px; width:100%; min-width:0; padding:2px; background:var(--bg-row); border-radius:var(--r-sm); }
  button { flex:1 1 0; min-width:0; height:24px; padding:0 4px; border:0; border-radius:calc(var(--r-sm) - 2px); background:transparent;
    color:var(--tx-3); font:inherit; font-size:var(--fs-sm); white-space:nowrap; overflow:hidden; text-overflow:ellipsis; }
  button:hover { background:var(--ink-1); color:var(--tx); }
  button:active { background:var(--ink-2); color:var(--tx); transform:scale(.94); transition:none; }
  button[aria-pressed="true"] { background:var(--bg-panel); color:var(--tx); box-shadow:none; }
  button:focus-visible { outline:2px solid var(--accent); outline-offset:-2px; }
</style>
