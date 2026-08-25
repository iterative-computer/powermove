<script lang="ts">
  import type { PanelProps } from './registerSveltePanel';
  import Composer from './agent/Composer.svelte';
  import Conversation from './agent/Conversation.svelte';
  import Steps from './agent/Steps.svelte';
  import { agentState } from './agent/agent-state.svelte';

  let { panelId }: PanelProps = $props();
  const PM = window.PM as Record<string, any>;
</script>

<div class="agent-panel-body agent-shell" data-svelte-panel={panelId} data-agent-panel data-agent-phase={agentState.phase}>
  <Steps {PM} />
  <Conversation {PM} />
  <Composer {PM} {panelId} />
</div>
