<script lang="ts">
  import Section from '../../controls/Section.svelte';
  import type { PowermoveAPI } from '../../kernel/api';
  import { FIGMA_COMMUNITY_URL, type FigmaSetupState, type FigmaLinkBridge } from '../../../../shared/figma-link';

  let { link, api, onDisconnect }: { link: FigmaLinkBridge; api: PowermoveAPI; onDisconnect: () => void } = $props();
  let connection = $state<FigmaSetupState>({ running: false, paired: false, pending: null });
  let preparing = $state(true), error = $state(''), copied = $state(false), working = $state(false);

  export function update(next: FigmaSetupState): void {
    connection = { ...next, manifestPath: next.manifestPath ?? connection.manifestPath };
  }
  export async function start(): Promise<void> {
    preparing = true;
    error = '';
    try {
      update(await link.start());
    } catch {
      error = 'Could not prepare Figma. Try again.';
    } finally {
      preparing = false;
    }
  }
  async function action(kind: 'copy' | 'show' | 'community'): Promise<void> {
    try {
      await link.pluginAction(kind);
      if (kind === 'copy') copied = true;
    } catch {
      error = 'Could not open the plugin. Try again.';
    }
  }
  async function decide(allow: boolean): Promise<void> {
    if (!connection.pending || working) return;
    working = true;
    error = '';
    try {
      await link.approve(connection.pending.id, allow);
    } catch (e) {
      error = e instanceof Error ? e.message : 'Try connecting again.';
    } finally {
      working = false;
    }
  }
  async function disconnect(): Promise<void> {
    try {
      await link.stop();
      onDisconnect();
    } catch {
      error = 'Could not disconnect. Try again.';
    }
  }
</script>

{#if connection.paired && !connection.pending && !connection.error}
  <Section {api} title="Figma Connected" />
  <p>Open a composition in Powermove. In Figma, select your layers and click Send to Powermove.</p>
  <button class="btn" onclick={disconnect}>Disconnect</button>
{:else}
  <Section {api} title="1 Add the Plugin" />
  {#if FIGMA_COMMUNITY_URL}
    <button class="btn" onclick={() => action('community')}>Open in Figma</button>
  {:else}
    <div class="actions">
      <button class="btn" disabled={preparing || !connection.manifestPath} onclick={() => action('copy')}>
        {copied ? 'Plugin Path Copied' : 'Copy Plugin Path'}
      </button>
      <button class="btn" disabled={preparing || !connection.manifestPath} onclick={() => action('show')}>Show in Finder</button>
    </div>
    <p>In Figma: Plugins → Development → Import plugin from manifest.</p>
    <p>In the file picker, press ⌘⇧G and paste the path. You only need to add the plugin once.</p>
  {/if}
  <Section {api} title="2 Connect" />
  {#if connection.pending}
    <p>Figma wants to connect.</p>
    <p class="connection-code" aria-label="Connection code">{connection.pending.code}</p>
    <p>Check that this code matches the one in Figma.</p>
    <div class="actions">
      <button class="btn" disabled={working} onclick={() => decide(false)}>Don't Allow</button>
      <button class="btn pri" disabled={working} onclick={() => decide(true)}>Allow</button>
    </div>
  {:else if preparing}
    <p role="status">Preparing the plugin…</p>
  {:else if connection.error}
    <p role="status">{connection.error}</p>
    <button class="btn" onclick={start}>Try Again</button>
  {:else}
    <p role="status">Open Send to Powermove in Figma and click Connect.</p>
  {/if}
{/if}
{#if error}<p role="alert">{error}</p>{/if}

<style>
  p { margin: 0; line-height: 1.5; color: var(--tx-2); }
  .actions { display: flex; gap: 8px; flex-wrap: wrap; }
  .connection-code { font-family: var(--f-mono); font-size: var(--fs-lg); letter-spacing: .15em; color: var(--tx); }
</style>
