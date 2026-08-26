<script lang="ts">
  import NumField from '../NumField.svelte';
  import Row from '../Row.svelte';
  import type { EditBinding } from '../gesture';

  let {
    PM,
    get,
    edit
  }: {
    PM: Record<string, any>;
    get: () => unknown;
    edit: EditBinding;
  } = $props();
</script>

<Row label="Opacity">
  <NumField {PM} {get} {edit} />
</Row>
