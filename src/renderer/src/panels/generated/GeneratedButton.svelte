<script lang="ts">
  import {
    formatPreview,
    generatedButtonDisabled,
    resolveButtonAction,
    type PreviewModel
  } from '../../core/generated-bindings';
  import type { GeneratedButtonControl } from '../../core/types/workspace';
  import { doc } from '../../state/document.svelte';
  import { sel } from '../../state/selection.svelte';

  let {
    PM,
    control,
    toolState,
    onPreview,
    onReset
  }: {
    PM: Record<string, any>;
    control: GeneratedButtonControl;
    toolState: Record<string, unknown>;
    onPreview: (preview: PreviewModel | null) => void;
    onReset: () => void;
  } = $props();

  let busy = $state(false);
  const disabled = $derived((
    doc.tick.structure,
    sel.layers,
    sel.keys,
    generatedButtonDisabled(PM, control, busy)
  ));

  async function run(): Promise<void> {
    if (busy || disabled) return;
    const action = control.action;
    if (action?.type === 'script') {
      busy = true;
      onPreview({
        ok: true,
        title: action.mode === 'apply' ? 'Running isolated tool…' : 'Preparing preview…',
        message: '',
        items: []
      });
    }
    try {
      const resolution = await resolveButtonAction(PM, control, $state.snapshot(toolState));
      if (resolution.kind === 'reset') onReset();
      else if (resolution.kind === 'preview') {
        onPreview(formatPreview(PM, resolution.result, resolution.applied));
        if (resolution.toast) PM.toast(resolution.toast);
      }
    } finally {
      busy = false;
    }
  }
</script>

<button
  type="button"
  class="chip"
  class:solid={control.primary}
  style="width:100%;justify-content:center;height:28px;margin-bottom:4px"
  {disabled}
  aria-busy={busy}
  onclick={run}
>{control.label}</button>
