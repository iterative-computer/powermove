<script>
  let layers = $state([{ id: 1, name: 'A' }, { id: 2, name: 'B' }, { id: 3, name: 'C' }]);
  let selected = $state(0);
</script>
<button data-action="reverse" onclick={() => layers = [...layers].reverse()}>Reverse</button>
<button data-action="remove" onclick={() => layers = layers.filter(layer => layer.id !== selected)}>Remove</button>
<ul>{#each layers as layer (layer.id)}
  <li data-id={layer.id} class:selected={selected === layer.id}>
    <button onclick={() => selected = layer.id}>{layer.name}</button>
  </li>
{/each}</ul>
<output>{selected}</output>
