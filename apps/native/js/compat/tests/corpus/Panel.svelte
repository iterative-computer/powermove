<script>
  let { api, panelId, spec } = $props();
  let name = $derived(api.project.latest().name);
</script>
<input aria-label="Project name" value={name} oninput={event => api.history.do('Panel rename', () => { api.host.pm.proj.name = event.currentTarget.value; })} />
<output data-panel={panelId}>{name}:{spec.label}</output>
