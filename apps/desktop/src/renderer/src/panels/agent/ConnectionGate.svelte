<script lang="ts">
  import type { ChatGPTAccountStatus } from '../../../../shared/ipc';
  import { agentState } from './agent-state.svelte';

  let { PM, status, busy, providerName, compact = false, connect, retry }: {
    PM: Record<string, any>;
    status: ChatGPTAccountStatus;
    busy: boolean;
    providerName: string;
    compact?: boolean;
    connect: () => Promise<void>;
    retry: () => Promise<void>;
  } = $props();

  const local = $derived(agentState.provider === 'compatible');
  const checking = $derived(status.state === 'checking');
  const waiting = $derived(status.state === 'connecting');
  const unavailable = $derived(status.state === 'unavailable');
  const heading = $derived(checking ? 'Checking connection…' : waiting ? `Finish connecting ${providerName}`
    : unavailable ? 'Connection needs attention'
    : local ? 'Bring your own model' : `Connect ${providerName}`);
</script>

<section class="agent-connect-gate" class:compact aria-label="Connect your agent">
  {#if !compact}
    <div class="connection-intro">
      <div class="connection-mark" aria-hidden="true">
        <svg viewBox="0 0 24 24"><path d="M10 3c.7 5.2 2.8 7.3 8 8-5.2.7-7.3 2.8-8 8-.7-5.2-2.8-7.3-8-8 5.2-.7 7.3-2.8 8-8Z" /><path d="M19 2v5m-2.5-2.5h5M20 16v5m-2.5-2.5h5" /></svg>
      </div>
      <div><h2>Make your next move</h2><p>Connect a model to start creating.</p></div>
    </div>
  {/if}
  <div class="connection-providers" role="group" aria-label="Connection provider">
    {#each agentState.providers as provider (provider.id)}
      <button type="button" aria-pressed={agentState.provider === provider.id}
        onclick={() => PM.AgentUI?.setProvider?.(provider.id)}
      >{provider.id === 'compatible' ? 'API / local' : provider.label}</button>
    {/each}
  </div>
  <div class="connection-card">
    <div class="connection-copy" role="status" aria-live="polite" aria-atomic="true">
      <h3>{heading}</h3>
      <p>{checking ? `Checking your ${providerName} connection.` : status.detail || (unavailable
        ? `Powermove could not start its ${providerName} service.`
        : local ? 'Use an API, Ollama, or LM Studio. Set up your model in settings.'
        : `Use your ${providerName} subscription to power the Powermove agent.`)}</p>
    </div>
    <button class="agent-connect-button" type="button" disabled={busy || waiting || checking}
      onclick={unavailable ? retry : connect}>
      {#if waiting || checking}<span class="connection-wait" aria-hidden="true"></span>{/if}
      <span>{checking ? 'Checking…' : waiting ? 'Waiting for sign-in…' : unavailable ? 'Try again' : local ? 'Open settings' : `Connect ${providerName}`}</span>
      {#if !waiting && !checking}<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3 8h10m-4-4 4 4-4 4" /></svg>{/if}
    </button>
  </div>
  {#if !local || unavailable}
    <button class="connection-settings" type="button" onclick={() => PM.SettingsUI?.open('accounts')}>Connection settings<svg viewBox="0 0 16 16" aria-hidden="true"><path d="m6 4 4 4-4 4" /></svg></button>
  {/if}
</section>

<style>
  .agent-connect-gate { display: flex; flex: none; flex-direction: column; gap: 12px; width: 100%; max-width: 360px; margin: auto; text-align: left; }
  .connection-intro { display: flex; align-items: center; gap: 10px; padding: 0 2px 2px; }
  .connection-mark { display: grid; place-items: center; flex: none; width: 34px; height: 38px; border-radius: 11px; background: color-mix(in srgb, var(--accent) 10%, transparent); color: var(--accent); }
  .connection-mark svg { width: 22px; height: 22px; fill: none; stroke: currentColor; stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; }
  h2 { margin: 0; color: var(--tx); font-size: var(--fs-md); font-weight: var(--fw-medium); line-height: 1.4; }
  .connection-intro p { margin-top: 2px; color: var(--tx-3); font-size: var(--fs-xs); line-height: 1.45; }
  .connection-providers { display: flex; gap: 2px; padding: 3px; border-radius: 9px; background: var(--bg-field); }
  .connection-providers button { flex: 1; min-width: 0; min-height: 28px; padding: 4px 5px; border-radius: 6px; color: var(--tx-3); font-size: var(--fs-xs); font-weight: var(--fw-medium); line-height: 1.3; }
  .connection-providers button:hover { color: var(--tx); background: var(--ink-1); }
  .connection-providers button[aria-pressed="true"] { color: var(--tx); background: var(--ctl-raised); box-shadow: var(--ctl-edge); }
  .connection-card { display: flex; flex-direction: column; gap: 14px; padding: 14px; border: 0; border-radius: 12px; background: var(--bg-field); }
  h3 { color: var(--tx); font-size: var(--fs-sm); font-weight: var(--fw-medium); line-height: 1.4; }
  .connection-copy p { margin-top: 5px; color: var(--tx-3); font-size: var(--fs-xs); line-height: 1.5; overflow-wrap: anywhere; }
  .agent-connect-button { display: flex; align-items: center; justify-content: center; gap: 8px; min-height: 34px; width: 100%; padding: 8px 10px; border-radius: 7px; background: var(--accent); color: var(--on-accent, #fff); font-size: var(--fs-sm); font-weight: var(--fw-medium); line-height: 1.3; }
  .agent-connect-button:hover { background: var(--accent-hover); }
  .agent-connect-button:active { transform: translateY(1px); }
  .agent-connect-button:disabled { opacity: 1; background: var(--ink-2); color: var(--tx-2); }
  .agent-connect-button svg, .connection-settings svg { width: 14px; height: 14px; flex: none; fill: none; stroke: currentColor; stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; }
  .connection-settings { display: flex; align-items: center; justify-content: center; align-self: center; gap: 3px; min-height: 24px; margin-top: -6px; padding: 3px 6px; border-radius: 5px; color: var(--tx-3); font-size: var(--fs-xs); }
  .connection-settings:hover { color: var(--tx); background: var(--ink-1); }
  button:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .connection-wait { flex: none; width: 10px; height: 10px; border: 1.5px solid var(--tx-4); border-top-color: var(--tx); border-radius: 50%; animation: connection-spin 1s linear infinite; }
  .compact { margin: 0 auto; gap: 8px; }
  @keyframes connection-spin { to { transform: rotate(360deg); } }
  @media (prefers-reduced-motion: reduce) { .connection-wait { animation: none; } }
</style>
