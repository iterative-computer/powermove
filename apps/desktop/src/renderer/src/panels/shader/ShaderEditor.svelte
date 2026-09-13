<script lang="ts">
  import { untrack } from 'svelte';
  import type { ShaderLayer } from '../../core/types/project';
  import { EditGesture, type EditBinding } from '../../controls/gesture';
  import { doc } from '../../state/document.svelte';
  import '../../controls/controls.css';
  import type { PowermoveAPI } from '../../kernel/api';

  let {
    PM,
    api,
    layer,
    panelId,
    onWrite,
    onCompile
  }: {
    PM: Record<string, any>;
    api: PowermoveAPI;
    layer: ShaderLayer;
    panelId: string;
    onWrite?: () => void;
    onCompile?: () => void;
  } = $props();

  /* The parent keys this component by layer id, so these must stay stable for
     the whole mount even when a document replacement swaps the layer object. */
  const edit: EditBinding = {
    mode: 'command',
    label: 'Edit shader',
    origin: 'shader-panel',
    command: (value) => ({ type: 'set_content', target: layer.id, patch: { code: String(value ?? '') } })
  };
  const gesture = new EditGesture(untrack(() => api), edit);
  const keyboardHelpId = `${untrack(() => panelId)}-shader-editor-keyboard-help`;

  let textareaElement: HTMLTextAreaElement;
  let draft = $state('');
  let live = false;
  let transactional = false;
  let lastWritten = '';
  let writeTimer: number | undefined;
  let tabExitArmed = false;

  $effect(() => {
    doc.tick.values;
    const next = layer.d.code;
    if (!live) {
      draft = next;
      lastWritten = next;
    }
  });

  $effect(() => () => {
    if (writeTimer !== undefined) window.clearTimeout(writeTimer);
    if (live) {
      writeNow(false);
      if (transactional) gesture.commit();
      live = false;
      transactional = false;
    }
  });

  function begin(): void {
    if (live) return;
    try {
      gesture.begin();
      transactional = true;
    } catch (error) {
      /* A stale legacy transaction must not make the editor eat keystrokes.
         One-shot writes remain available until the next focus gesture. */
      transactional = false;
      window.console.warn('Shader edit transaction unavailable; using one-shot writes', error);
    } finally {
      live = true;
      lastWritten = layer.d.code;
    }
  }

  function scheduleWrite(): void {
    if (writeTimer !== undefined) window.clearTimeout(writeTimer);
    writeTimer = window.setTimeout(() => writeNow(true), 420);
  }

  function writeNow(continueGesture = false): void {
    if (writeTimer !== undefined) window.clearTimeout(writeTimer);
    writeTimer = undefined;
    if (!live || draft === lastWritten) return;
    if (transactional) gesture.write(draft);
    else gesture.once(draft);
    lastWritten = draft;
    PM.invalidate?.();
    onWrite?.();
    /* Legacy created one undo entry per 420ms burst. We retain the focus
       gesture but split completed debounce bursts; blur-only flushes remain a
       single final burst rather than opening an empty replacement gesture. */
    if (continueGesture && transactional) {
      gesture.commit();
      try {
        gesture.begin();
      } catch (error) {
        transactional = false;
        window.console.warn('Shader edit transaction could not continue; using one-shot writes', error);
      }
    }
  }

  function input(): void {
    draft = textareaElement.value;
    tabExitArmed = false;
    scheduleWrite();
  }

  function blur(): void {
    if (!live) return;
    writeNow(false);
    if (transactional) gesture.commit();
    live = false;
    transactional = false;
    tabExitArmed = false;
  }

  function keydown(event: KeyboardEvent): void {
    if (event.key === 'Tab' && tabExitArmed) {
      tabExitArmed = false;
      event.stopPropagation();
      return;
    }

    event.stopPropagation();

    if (event.key === 'Escape') {
      tabExitArmed = true;
      return;
    }

    tabExitArmed = false;
    if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
      event.preventDefault();
      writeNow(false);
      onCompile?.();
      return;
    }

    if (event.key !== 'Tab') return;
    event.preventDefault();
    const start = textareaElement.selectionStart;
    textareaElement.setRangeText('  ', start, textareaElement.selectionEnd, 'end');
    draft = textareaElement.value;
    scheduleWrite();
  }
</script>

<div class="shader-editor">
  <textarea
    bind:this={textareaElement}
    class="code"
    data-shader-editor
    aria-label="Shader source code"
    aria-describedby={keyboardHelpId}
    spellcheck={false}
    value={draft}
    onfocus={begin}
    oninput={input}
    onblur={blur}
    onkeydown={keydown}
  ></textarea>
  <span id={keyboardHelpId} class="panel-sr-only">
    Tab inserts two spaces. Press Escape, then Tab to leave the editor.
  </span>
</div>

<style>
  .shader-editor {
    position: relative;
    display: flex;
    flex: 1;
    min-height: 0;
  }

  textarea.code {
    flex: 1;
    min-height: 0;
  }
</style>
