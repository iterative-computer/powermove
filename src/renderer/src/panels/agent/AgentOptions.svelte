<script lang="ts">
  import PanelFocusPicker from './PanelFocusPicker.svelte';
  import ModelPicker from './ModelPicker.svelte';
  let { PM }: { PM: Record<string, any> } = $props();
</script>
<div class="agent-option-bar">
  <PanelFocusPicker {PM} />
  <span class="sp"></span>
  <ModelPicker {PM} />
</div>
