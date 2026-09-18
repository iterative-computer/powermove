<script lang="ts">
  import { doc } from '../state/document.svelte';
  import Icon from './Icon.svelte';
  import type { PanelProps } from './registerSveltePanel';

  interface Workspace {
    id: string;
    name: string;
    builtin?: boolean;
    theme?: { accent?: string };
  }

  let { panelId }: PanelProps = $props();

  const PM = window.PM as Record<string, any>;
  let workspaceVersion = $state(0);
  let status = $state('');
  const model = $derived.by(() => {
    workspaceVersion;
    doc.proj;
    return {
      workspaces: [...((PM.WS.all ?? []) as Workspace[])],
      currentId: PM.WS.current?.id as string | undefined
    };
  });

  $effect(() => {
    /* Phase 5 removal: workspace state has no rune/tick yet, so this is the
       sole temporary legacy-bus-to-local-rune bridge in these panels. */
    const offWorkspaces = PM.bus.on('workspaces', () => {
      workspaceVersion += 1;
    });
    const offLayout = PM.bus.on('layout', () => {
      workspaceVersion += 1;
    });
    return () => {
      offWorkspaces?.();
      offLayout?.();
    };
  });

  function activate(workspace: Workspace): void {
    PM.WS.activate(workspace.id);
    status = `Activated ${workspace.name}`;
  }

  function remove(event: MouseEvent, workspace: Workspace): void {
    event.stopPropagation();
    PM.modal?.({
      title: `Delete “${workspace.name}” workspace?`,
      body: 'This removes the saved workspace layout. Your project and its layers will not be deleted.',
      actions: [
        { label: 'Cancel' },
        {
          label: 'Delete workspace',
          pri: true,
          run: () => {
            PM.WS.remove(workspace.id);
            status = `Deleted ${workspace.name}`;
          }
        }
      ]
    });
  }

  function saveCurrent(): void {
    PM.WS.saveAsNew();
    status = 'Save workspace dialog opened';
  }

  function editJson(): void {
    PM.WS.editJSON();
    status = 'Workspace JSON editor opened';
  }
</script>

<div class="simple-panel-list" data-svelte-panel={panelId}>
  {#each model.workspaces as workspace (workspace.id)}
    <div class:sel={workspace.id === model.currentId} class="lyr simple-panel-row">
      <button
        class="simple-row-action"
        type="button"
        aria-current={workspace.id === model.currentId ? 'true' : undefined}
        onclick={() => activate(workspace)}
      >
        <span class="nm" title={workspace.name}>{workspace.name}</span>
      </button>
      {#if !workspace.builtin}
        <button class="stopwatch" type="button" aria-label={`Delete ${workspace.name}`} title={`Delete ${workspace.name} workspace`} onclick={(event) => remove(event, workspace)}>
          <Icon {PM} name="trash" />
        </button>
      {/if}
    </div>
  {/each}
  <div class="simple-panel-actions">
    <button class="chip" type="button" onclick={saveCurrent}>
      <Icon {PM} name="plus" />
      Save current
    </button>
    <button class="chip ghost quiet" type="button" title="Edit workspace layout as JSON" onclick={editJson}>
      <Icon {PM} name="code" />
      JSON
    </button>
  </div>
  <div class="simple-panel-hint">Shake the pointer, then drag across a panel to redesign or add an interface section.</div>
  <span class="panel-sr-only" role="status">{status}</span>
</div>
