<script lang="ts">
  import type { SlashCommand } from './slash-commands';
  let { id, anchor, options, selected, choose, dismiss }: {
    id: string; anchor: HTMLElement; options: SlashCommand[]; selected: number;
    choose: (option: SlashCommand) => void;
    dismiss: () => void;
  } = $props();
  let menu: HTMLDivElement;
  let left = $state(0);
  let bottom = $state(0);
  let width = $state(280);
  let maxHeight = $state(260);
  $effect(() => {
    const position = () => {
      const rect = (anchor.closest('.agent-composer') || anchor).getBoundingClientRect();
      width = Math.min(Math.max(220, rect.width), window.innerWidth - 16);
      left = Math.max(8, Math.min(rect.left, window.innerWidth - width - 8));
      bottom = window.innerHeight - rect.top + 6;
      maxHeight = Math.max(80, Math.min(280, rect.top - 16));
    };
    position();
    menu?.showPopover?.();
    const outside = (event: PointerEvent) => {
      if (event.target instanceof Node && !menu.contains(event.target) && !anchor.closest('.agent-composer')?.contains(event.target)) dismiss();
    };
    window.addEventListener('pointerdown', outside);
    window.addEventListener('resize', position);
    window.addEventListener('scroll', position, true);
    return () => {
      window.removeEventListener('pointerdown', outside);
      window.removeEventListener('resize', position);
      window.removeEventListener('scroll', position, true);
    };
  });
  $effect(() => { menu?.querySelector<HTMLElement>(`[data-index="${selected}"]`)?.scrollIntoView({ block: 'nearest' }); });
</script>

<!-- Keep typing focus in the composer while choosing with the pointer. -->
<div bind:this={menu} {id} class="agent-slash-menu" popover="manual" role="listbox" aria-label="Slash commands"
  style:left={`${left}px`} style:bottom={`${bottom}px`} style:width={`${width}px`} style:max-height={`${maxHeight}px`}>
  {#each options as option, index (option.id)}
    <button type="button" role="option" id={`${id}-${index}`} data-index={index} tabindex="-1"
      aria-selected={index === selected} onpointerdown={event => event.preventDefault()} onclick={() => choose(option)}>
      <span>{option.label}</span><small>{option.description}</small>
    </button>
  {/each}
</div>

<style>
  .agent-slash-menu { position: fixed; inset: auto; margin: 0; padding: 5px; box-sizing: border-box; overflow-y: auto; border: 0; border-radius: var(--r-lg); background: var(--bg-float); box-shadow: var(--shadow-float); color: var(--tx); font: var(--fs-sm)/1.5 var(--f-ui); }
  button { display: flex; flex-direction: column; gap: 1px; width: 100%; min-width: 0; padding: 7px 9px; border: 0; border-radius: var(--r-sm); background: transparent; color: inherit; font: inherit; text-align: left; cursor: pointer; overflow-wrap: anywhere; }
  button[aria-selected="true"], button:hover { background: var(--ink-2); }
  button:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
  small { color: var(--tx-3); font-size: var(--fs-xs); }
</style>
