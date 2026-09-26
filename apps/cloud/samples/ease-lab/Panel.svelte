<script lang="ts">
  import { onMount } from 'svelte';
  import type { PanelProps } from 'powermove';

  let { api }: PanelProps = $props();
  let preset = $state('ease-in-out');
  let canvas: HTMLCanvasElement;
  const presets = [
    { id: 'linear', label: 'Linear' },
    { id: 'ease-in', label: 'Ease in' },
    { id: 'ease-out', label: 'Ease out' },
    { id: 'ease-in-out', label: 'Ease in and out' },
  ];

  function value(t: number): number {
    if (preset === 'ease-in') return t * t;
    if (preset === 'ease-out') return 1 - (1 - t) * (1 - t);
    if (preset === 'ease-in-out') return t * t * (3 - 2 * t);
    return t;
  }

  function draw(): void {
    const context = canvas?.getContext('2d');
    if (!context) return;
    const { width, height } = canvas;
    context.clearRect(0, 0, width, height);
    context.strokeStyle = '#72818f';
    context.lineWidth = 1;
    context.strokeRect(12, 12, width - 24, height - 24);
    context.beginPath();
    context.strokeStyle = '#72c6ec';
    context.lineWidth = 3;
    for (let x = 0; x <= 100; x++) {
      const t = x / 100;
      const px = 12 + t * (width - 24);
      const py = height - 12 - value(t) * (height - 24);
      if (x === 0) context.moveTo(px, py);
      else context.lineTo(px, py);
    }
    context.stroke();
  }

  $effect(() => { preset; draw(); });

  onMount(() => {
    void (async () => {
      const saved = await api!.storage.get<string>('preset');
      if (presets.some((item) => item.id === saved)) preset = saved!;
    })();
  });

  async function remember(): Promise<void> {
    await api!.storage.set('preset', preset);
  }

  async function apply(): Promise<void> {
    await api!.commands.run(`${api!.id}.apply`);
  }
</script>

<section class="lab">
  <label for="ease-lab-preset">Easing preset</label>
  <select id="ease-lab-preset" bind:value={preset} onchange={remember}>
    {#each presets as item}
      <option value={item.id}>{item.label}</option>
    {/each}
  </select>
  <canvas bind:this={canvas} width="256" height="144" aria-label="Preview of the selected easing curve"></canvas>
  <button type="button" onclick={apply}>Apply</button>
</section>

<style>
  .lab { display: flex; flex-direction: column; gap: 10px; padding: 12px; color: var(--tx); }
  label { color: var(--tx-2); }
  select, button { min-height: 32px; padding: 5px 8px; border: 1px solid var(--ink-3); border-radius: 5px; background: var(--bg-field); color: var(--tx); font: inherit; }
  select:focus-visible, button:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  canvas { width: 100%; height: auto; border-radius: 5px; background: var(--bg-field); }
  button { cursor: pointer; }
</style>
