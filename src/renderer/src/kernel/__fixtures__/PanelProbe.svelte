<script lang="ts">
  /* Test fixture: the smallest component that proves `panelId` and `spec`
     reached a kernel-registered panel through the host's mount path. */
  let { panelId, spec }: { panelId: string; spec: Record<string, unknown> } = $props();
</script>

<span data-probe>{panelId}:{spec.note ?? ''}</span>
