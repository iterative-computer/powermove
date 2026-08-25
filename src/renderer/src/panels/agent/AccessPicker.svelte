<script lang="ts">
  import { onDestroy } from 'svelte';
  import Icon from '../Icon.svelte';
  import { agentState } from './agent-state.svelte';

  let { PM }: { PM: Record<string, any> } = $props();
  let select = $state<HTMLSelectElement>();
  let confirmation: { el: HTMLElement; close(): void } | null = null;
  let appWasInert = false;
  const current = $derived(agentState.accessModes.find((mode) => mode.id === agentState.accessMode) ?? agentState.accessModes[0]);

  function change(event: Event): void {
    const next = (event.currentTarget as HTMLSelectElement).value;
    if (next === 'computer') {
      (event.currentTarget as HTMLSelectElement).value = agentState.accessMode;
      openComputerConfirmation();
      return;
    }
    PM.AgentUI?.setAccess(next);
  }

  function closeConfirmation(): void {
    confirmation?.close();
  }

  function modalKeydown(event: KeyboardEvent): void {
    if (!confirmation) return;
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      closeConfirmation();
      return;
    }
    if (event.key !== 'Tab') return;
    const buttons = [...confirmation.el.querySelectorAll('button')] as HTMLButtonElement[];
    if (!buttons.length) return;
    const first = buttons[0]!;
    const last = buttons[buttons.length - 1]!;
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }

  function openComputerConfirmation(): void {
    if (confirmation) return;
    const body = PM.h('div.agent-access-warning',
      PM.h('p', 'Codex can read or change files outside this project, launch applications, and use services already signed in on this Mac.'),
      PM.h('p', 'External actions cannot be undone. Computer access resets when this run finishes or Powermove quits.')
    );
    const app = document.querySelector<HTMLElement>('#app');
    appWasInert = app?.inert ?? false;
    const handle = PM.modal({
      title: 'Allow computer access for this run?',
      body,
      width: 390,
      actions: [
        { label: 'Cancel' },
        { label: 'Allow for one run', pri: true, run: () => PM.AgentUI?.confirmComputerAccess() }
      ],
      onClose: () => {
        if (app) app.inert = appWasInert;
        confirmation = null;
        queueMicrotask(() => select?.focus());
      }
    });
    confirmation = handle;
    if (app) app.inert = true;
    const title = handle.el.querySelector('h3');
    if (title) title.id = 'agent-computer-access-title';
    handle.el.setAttribute('role', 'alertdialog');
    handle.el.setAttribute('aria-modal', 'true');
    handle.el.setAttribute('aria-labelledby', 'agent-computer-access-title');
    handle.el.addEventListener('keydown', modalKeydown);
    queueMicrotask(() => (confirmation?.el.querySelector('button') as HTMLButtonElement | null)?.focus());
  }

  function selectKeydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === 'Escape') select?.blur();
  }

  onDestroy(closeConfirmation);
</script>

<label class="agent-access" title={current?.detail ?? ''}>
  <Icon {PM} name={agentState.accessMode === 'editor' ? 'panel' : 'sparkle'} />
  <select bind:this={select} aria-label="Agent authority" value={agentState.accessMode} onchange={change} onkeydown={selectKeydown}>
    {#each agentState.accessModes as mode (mode.id)}
      <option value={mode.id}>{mode.label}</option>
    {/each}
  </select>
  <Icon {PM} name="chev" />
</label>
