<script lang="ts">
  import { untrack } from 'svelte';
  import { bridge } from '../kernel/bridge';
  import type { AgentClientId, AgentClientsInfo } from '../../../shared/agent-clients';

  /* Any agent app that supports MCP can drive Powermove on its own plan.
     Apps found on this Mac get Powermove added in one click; for anything
     else, Copy setup gives the standard MCP entry to paste. */
  let { active }: { active: boolean } = $props();
  const host = bridge()?.agentClients;
  let info = $state<AgentClientsInfo | null>(null);
  let busy = $state<AgentClientId | null>(null);
  let notes = $state<Partial<Record<AgentClientId, string>>>({});
  let error = $state('');
  let copied = $state(false);
  const shown = $derived(info?.clients.filter(client => client.state !== 'not-installed') ?? []);

  const message = (cause: unknown, fallback: string): string =>
    cause instanceof Error && cause.message ? cause.message.replace(/^Error invoking remote method '[^']+': (?:Error: )?/, '') : fallback;

  async function refresh(): Promise<void> {
    if (!host) return;
    try { info = await host.list(); error = ''; }
    catch (cause) { error = message(cause, 'Could not check your agent apps.'); }
  }

  async function change(id: AgentClientId, add: boolean): Promise<void> {
    if (!host || busy) return;
    busy = id;
    error = '';
    try {
      const next = add ? await host.connect(id) : (await host.disconnect(id), 'Removed.');
      notes = { ...notes, [id]: next };
      await refresh();
    } catch (cause) {
      notes = { ...notes, [id]: message(cause, 'Could not change this app’s settings.') };
    } finally { busy = null; }
  }

  /** The standard MCP entry most apps accept as-is. */
  function setup(): string {
    if (!info) return '';
    return JSON.stringify({ mcpServers: { powermove: { command: info.command, args: info.args } } }, null, 2);
  }

  async function copySetup(): Promise<void> {
    try {
      const text = setup();
      const write = bridge()?.clipboardWriteText;
      if (write) await write(text); else await navigator.clipboard.writeText(text);
      copied = true;
      setTimeout(() => { copied = false; }, 1600);
    } catch (cause) { error = message(cause, 'Could not copy. Try again.'); }
  }

  $effect(() => { if (active && host) untrack(() => { notes = {}; void refresh(); }); });
</script>

{#if host}
  <section class="sg-section">
    <h3 class="sg-section-title">Other agent apps</h3>
    <div class="sg-group" aria-busy={busy !== null}>
      {#each shown as client (client.id)}
        <div class="settings-row settings-provider">
          <div class="settings-copy">
            <b class="settings-provider-title"><span>{client.name}</span><i class="settings-dot" aria-hidden="true" data-state={client.state === 'connected' ? 'connected' : 'off'}></i></b>
            <span>{notes[client.id] ?? (client.state === 'unreadable' ? 'Its settings can’t be edited automatically. Use Copy setup below.'
              : client.custom ? 'Powermove is set up in this app.' : client.detail)}</span>
          </div>
          {#if client.state === 'connected'}
            <button class="btn" type="button" disabled={busy !== null || client.custom}
              onclick={() => void change(client.id, false)}>{client.custom ? 'Added' : busy === client.id ? 'Removing…' : 'Remove'}</button>
          {:else if client.state === 'available'}
            <button class="btn" type="button" disabled={busy !== null}
              onclick={() => void change(client.id, true)}>{busy === client.id ? 'Adding…' : 'Add Powermove'}</button>
          {/if}
        </div>
      {/each}
      <div class="settings-row settings-provider">
        <div class="settings-copy">
          <b>Any other agent</b>
          <span>Paste into any app that supports MCP servers, such as Cline, Roo Code or Kiro. It runs on that app’s own plan.</span>
        </div>
        <button class="btn" type="button" disabled={!info} onclick={() => void copySetup()}>{copied ? 'Copied' : 'Copy setup'}</button>
      </div>
    </div>
    {#if error}<p class="settings-note" role="status">{error}</p>{/if}
  </section>
{/if}
