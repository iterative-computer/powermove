<script>
  let name = $state('Layer');
  let enabled = $state(true);
  let mode = $state('normal');
</script>
<section aria-label="Inspector">
  <input aria-label="Name" bind:value={name} />
  <input type="checkbox" aria-label="Enabled" bind:checked={enabled} />
  <select aria-label="Blend" bind:value={mode}>
    <option value="normal">Normal</option><option value="multiply">Multiply</option>
  </select>
  <output style:opacity={enabled ? 1 : 0.5}>{name}: {mode}</output>
</section>
