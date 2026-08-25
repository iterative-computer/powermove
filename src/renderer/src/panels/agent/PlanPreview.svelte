<script lang="ts">
  import Icon from '../Icon.svelte';
  import { agentState, describePanelAction } from './agent-state.svelte';

  let { PM }: { PM: Record<string, any> } = $props();

  const plan = $derived(agentState.plan);
  const title = $derived.by(() => {
    if (!plan) return '';
    if (plan.kind === 'panels') return 'Panel arrangement';
    if (plan.kind === 'interface') return 'Timeline redesign';
    if (plan.kind === 'chrome') return 'Interface edit';
    if (plan.kind === 'scene') return plan.sceneEdit.label;
    if (plan.kind === 'workspace') return plan.workspaceEdit.name;
    return plan.section.title;
  });

  const applyLabel = $derived(plan?.kind === 'panels' ? 'Apply panel changes'
    : ['chrome', 'interface'].includes(plan?.kind) ? 'Apply interface edit'
      : plan?.kind === 'scene' ? 'Apply scene edit'
        : plan?.kind === 'workspace' ? 'Create workspace' : 'Apply section');

</script>

{#if plan}
  <div class="spatial-proposal">
    <div class="spatial-proposal-kicker">Proposed change</div>
    <h3>{title}</h3>
    {#if plan.kind === 'panels'}
      {#each plan.panelEdit.actions as action}
        <div class="spatial-preview-control panel-action"><Icon {PM} name="panel" /><span>{describePanelAction(PM, action)}</span></div>
      {/each}
    {:else if plan.kind === 'interface'}
      {#each Object.entries(plan.interfaceEdit.patch) as [key, value]}
        <div class="spatial-preview-control"><span>{key}</span><span>{String(value)}</span></div>
      {/each}
    {:else if plan.kind === 'chrome'}
      <div class="spatial-preview-control">
        <span>{plan.chromeEdit.target === 'timeline.surfaceOrder' ? 'Timeline surfaces' : 'Corner style'}</span>
        <span>{plan.chromeEdit.value}</span>
      </div>
    {:else if plan.kind === 'scene'}
      {#each plan.sceneEdit.commands as command}
        <div class="spatial-preview-control"><span>{PM.AgentHarness.describeCommand(command)}</span></div>
      {/each}
    {:else if plan.kind === 'workspace'}
      {#each plan.workspaceEdit.docks as dock}
        <div class="spatial-preview-control"><span>{dock.id}</span><span>{dock.panels.map((panel: any) => panel.id).join(' · ')}</span></div>
      {/each}
      {#each plan.workspaceEdit.sections as section}
        <div class="spatial-preview-control"><span>{section.title}</span><span>{section.controls.length} connected controls</span></div>
      {/each}
    {:else}
      {#each plan.section.controls as control}
        <div class="spatial-preview-control">
          <span>{control.label}</span><span>{control.connection || control.type}</span>
          {#if ['slider', 'curve'].includes(control.type)}<i class={control.type} aria-hidden="true"></i>{/if}
        </div>
      {/each}
    {/if}
    <div class="spatial-proposal-actions">
      <button class="spatial-action" type="button" onclick={() => PM.AgentUI?.dismissPlan()}>Dismiss</button>
      <button class="spatial-action pri" type="button" onclick={() => PM.AgentUI?.applyPlan()}>{applyLabel}</button>
    </div>
  </div>
{/if}
