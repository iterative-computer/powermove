<script lang="ts">
  import AccessPicker from './AccessPicker.svelte';
  import ModelPicker from './ModelPicker.svelte';
  let { PM }: { PM: Record<string, any> } = $props();
</script>
<div class="agent-option-bar">
  <ModelPicker {PM} />
  <span class="sp"></span>
  <AccessPicker {PM} />
</div>
