<script lang="ts">
  import { agentState } from './agent-state.svelte';
  import { openPopoverMenu } from '../../controls/popover-menu';

  let { PM }: { PM: Record<string, any> } = $props();

  /* How far the agent may reach and who approves what the sandbox blocks.
     Rows carry their own icon and one line of what they allow. */
  const current = $derived(agentState.accessModes.find(mode => mode.id === agentState.permission));

  function open(event: MouseEvent): void {
    const header = document.createElement('div');
    header.className = 'agent-access-head';
    header.textContent = 'How should agent actions be approved?';
    openPopoverMenu({
      anchor: event.currentTarget as HTMLElement,
      label: 'Agent access',
      header,
      side: 'top',
      align: 'end',
      items: agentState.accessModes.map(mode => ({
        label: mode.label,
        ...(mode.detail ? { detail: mode.detail } : {}),
        ...(mode.icon ? { icon: PM.icon(mode.icon) } : {}),
        ...(mode.tone ? { tone: mode.tone } : {}),
        checked: mode.id === agentState.permission,
        run: () => PM.AgentUI?.setAccess(mode.id)
      }))
    });
  }
</script>

{#if current}
  <button class="agent-access" type="button" data-tone={current.tone}
    aria-label={`Agent access: ${current.label}`} title={current.detail} onclick={open}
    onkeydown={(event) => event.stopPropagation()}>
    <span>{current.label}</span>
  </button>
{/if}
