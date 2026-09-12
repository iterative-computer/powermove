<script lang="ts">
  import { doc } from '../state/document.svelte';
  import type { PanelProps } from './registerSveltePanel';

  let { panelId }: PanelProps = $props();

  const PM = window.PM as Record<string, any>;
  let notes = $state(doc.proj?.notes ?? '');
  let focused = $state(false);
  let status = $state('');
  let saveTimer: number | undefined;
  let pendingProject: Record<string, any> | undefined;
  let pendingNotes = '';
  let renderedProject = doc.proj;
  let renderedProjectTick = doc.tick.project;
  let textareaElement: HTMLTextAreaElement;

  $effect(() => {
    const tick = doc.tick.project;
    const project = doc.proj;
    if (project !== renderedProject) {
      // Finish the old project's pending write before showing the replacement.
      flushNotes();
      renderedProject = project;
      notes = project?.notes ?? '';
    } else if (tick !== renderedProjectTick && !focused) {
      notes = project?.notes ?? '';
    }
    renderedProjectTick = tick;
  });

  $effect(() => () => flushNotes());
  $effect(() => PM.bus?.on?.('project:flush-edits', flushNotes));

  $effect(() => {
    const element = textareaElement;
    element.addEventListener('keydown', stopPanelShortcuts);
    return () => element.removeEventListener('keydown', stopPanelShortcuts);
  });

  function persistNotes(): void {
    saveTimer = undefined;
    const project = pendingProject;
    const value = pendingNotes;
    pendingProject = undefined;
    if (!project || project.notes === value) return;
    /* Phase 4 exception: set_composition rejects `notes` in both the typed
       validator and legacy editing allowlist. Remove this direct write once
       Phase 5 gives project notes a source-edit command. */
    if (PM.proj === project) PM.proj.notes = value;
    else project.notes = value;
    if (PM.proj === project) PM.autosave?.();
    status = 'Notes saved';
  }

  function scheduleSave(): void {
    if (saveTimer !== undefined) window.clearTimeout(saveTimer);
    pendingProject = PM.proj;
    pendingNotes = notes;
    status = 'Saving notes…';
    saveTimer = window.setTimeout(persistNotes, 250);
  }

  function flushNotes(): void {
    if (saveTimer === undefined) return;
    window.clearTimeout(saveTimer);
    persistNotes();
  }

  function handleFocus(): void {
    focused = true;
  }

  function handleInput(event: Event): void {
    notes = (event.currentTarget as HTMLTextAreaElement).value;
    scheduleSave();
  }

  function handleBlur(): void {
    focused = false;
    flushNotes();
  }

  function stopPanelShortcuts(event: KeyboardEvent): void {
    event.stopPropagation();
  }
</script>

<div class="notes-panel" data-svelte-panel={panelId}>
  <textarea
    aria-label="Project notes"
    placeholder="Direction notes — saved with this project as its creative brief."
    value={notes}
    bind:this={textareaElement}
    onfocus={handleFocus}
    oninput={handleInput}
    onblur={handleBlur}
  ></textarea>
  <span class="panel-sr-only" role="status">{status}</span>
</div>
