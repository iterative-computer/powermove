<script lang="ts">
  import NumField from '../NumField.svelte';
  import Row from '../Row.svelte';
  import type { EditBinding } from '../gesture';
  import type { PowermoveAPI } from '../../kernel/api';

  let {
    api,
    get,
    edit
  }: {
    api: PowermoveAPI;
    get: () => unknown;
    edit: EditBinding;
  } = $props();
</script>

<Row {api} label="Opacity">
  <NumField {api} {get} {edit} />
</Row>
