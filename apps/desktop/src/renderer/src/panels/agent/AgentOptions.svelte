<script lang="ts">
  import ModelPicker from './ModelPicker.svelte';
  let { PM }: { PM: Record<string, any> } = $props();
</script>
<div class="agent-option-bar">
  <ModelPicker {PM} />
</div>
