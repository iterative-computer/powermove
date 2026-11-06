<script lang="ts">
  /* Generic segmented control. role=tablist with roving tabindex; arrow keys
     move and select, Home/End jump. The selected segment carries class `on`. */
  export interface SegmentOption {
    id: string;
    label: string;
  }

  let {
    options,
    value,
    onChange,
    label
  }: { options: SegmentOption[]; value: string; onChange: (id: string) => void; label?: string } = $props();

  let track: HTMLDivElement | undefined = $state();

  function pick(index: number): void {
    const next = options[(index + options.length) % options.length];
    if (!next) return;
    onChange(next.id);
    track?.querySelectorAll<HTMLButtonElement>('button')[(index + options.length) % options.length]?.focus();
  }

  function onKey(event: KeyboardEvent, index: number): void {
    const map: Record<string, number> = {
      ArrowLeft: index - 1,
      ArrowUp: index - 1,
      ArrowRight: index + 1,
      ArrowDown: index + 1,
      Home: 0,
      End: options.length - 1
    };
    const target = map[event.key];
    if (target === undefined) return;
    event.preventDefault();
    pick(target);
  }
</script>

<div class="segmented" role="tablist" aria-label={label} bind:this={track}>
  {#each options as option, index (option.id)}
    <button
      type="button"
      role="tab"
      class:on={option.id === value}
      aria-selected={option.id === value}
      tabindex={option.id === value ? 0 : -1}
      onclick={() => onChange(option.id)}
      onkeydown={(event) => onKey(event, index)}
    >{option.label}</button>
  {/each}
</div>
