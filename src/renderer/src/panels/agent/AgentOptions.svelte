<script lang="ts">
  import PanelFocusPicker from './PanelFocusPicker.svelte';
  import ModelPicker from './ModelPicker.svelte';
  let { PM, showPanelFocus = true }: { PM: Record<string, any>; showPanelFocus?: boolean } = $props();
</script>
<div class="agent-option-bar">
  {#if showPanelFocus}
    <PanelFocusPicker {PM} />
    <span class="sp"></span>
  {/if}
  <ModelPicker {PM} />
</div>
