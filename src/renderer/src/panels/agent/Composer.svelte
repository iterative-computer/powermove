<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from '../Icon.svelte';
  import AttachmentChips from './AttachmentChips.svelte';
  import { ATTACHMENT_HINT } from './attachments';
  import { agentState, composerMode } from './agent-state.svelte';
  import AgentOptions from './AgentOptions.svelte';

  let { PM, panelId }: { PM: Record<string, any>; panelId: string } = $props();
  let textarea: HTMLTextAreaElement;
  let fileInput: HTMLInputElement;
  let draft = $state('');
  let dragDepth = $state(0);
  let lastFocusVersion = 0;
  const mode = $derived(composerMode(agentState.legacyPhase));
  const textareaId = $derived(`agent-composer-${panelId}`);

  $effect(() => {
    agentState.revision;
    const next = agentState.composerDraft;
    if (draft !== next) draft = next;
    if (textarea && textarea.value !== next) textarea.value = next;
    if (textarea) queueMicrotask(autosize);
  });

  $effect(() => {
    const next = agentState.focusVersion;
    if (next !== lastFocusVersion && !mode.disabled) {
      lastFocusVersion = next;
      queueMicrotask(() => {
        if (!document.querySelector('.spatial-compose')) textarea?.focus();
      });
    }
  });

  function autosize(): void {
    if (!textarea) return;
    /* Blank the placeholder before measuring: Chromium counts the wrapped
       placeholder in scrollHeight, which locks an empty box open. */
    const placeholder = textarea.placeholder;
    if (!textarea.value) textarea.placeholder = '';
    textarea.style.height = '24px';
    const style = window.getComputedStyle(textarea);
    const layout = PM.SpatialAssistant.math.textareaLayout(
      textarea.scrollHeight,
      Number.parseFloat(style.minHeight),
      Number.parseFloat(style.maxHeight)
    );
    textarea.style.height = `${layout.height}px`;
    textarea.style.overflowY = layout.overflowY;
    textarea.placeholder = placeholder;
  }

  function input(): void {
    draft = textarea.value;
    PM.AgentUI?.setDraft(draft);
    autosize();
  }

  function submit(): void {
    if (!textarea.value.trim() && !agentState.attachments.length) return;
    PM.AgentUI?.submit(textarea.value);
  }

  function keydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if ((event.metaKey || event.ctrlKey) && !event.altKey && event.key.toLowerCase() === 'a') {
      /* Keep select-all local to the draft. Powermove also owns this chord for
         layer selection, and Electron's native menu routing can otherwise win
         before Chromium applies the textarea default. */
      event.preventDefault();
      textarea.select();
      return;
    }
    if (event.key === 'Enter' && !event.shiftKey) {
      event.preventDefault();
      submit();
    }
  }

  function filesFromTransfer(transfer: DataTransfer | null): File[] {
    const direct = [...(transfer?.files || [])].filter((file) => file instanceof window.File);
    if (direct.length) return direct;
    return [...(transfer?.items || [])]
      .filter((item) => item.kind === 'file').map((item) => item.getAsFile()).filter((file): file is File => !!file);
  }

  function paste(event: ClipboardEvent): void {
    const files = filesFromTransfer(event.clipboardData);
    if (!files.length) return;
    event.preventDefault();
    void PM.AgentUI?.addAttachments(files);
  }

  function drop(event: DragEvent): void {
    event.preventDefault();
    dragDepth = 0;
    const files = filesFromTransfer(event.dataTransfer);
    if (files.length) void PM.AgentUI?.addAttachments(files);
  }

  onMount(() => queueMicrotask(autosize));
</script>

<div
  class="agent-composer"
  class:is-dropping={dragDepth > 0}
  role="group"
  aria-label="Message composer"
  ondragenter={(event) => { event.preventDefault(); dragDepth += 1; }}
  ondragover={(event) => event.preventDefault()}
  ondragleave={() => { dragDepth = Math.max(0, dragDepth - 1); }}
  ondrop={drop}
>
  {#if agentState.attachments.length}
    <div class="agent-attachment-rail">
      <AttachmentChips {PM} items={agentState.attachments} removable onRemove={(id) => PM.AgentUI?.removeAttachment(id)} />
    </div>
  {/if}
  <div class="agent-input-row">
    <input class="panel-sr-only" bind:this={fileInput} type="file" multiple onchange={() => { if (fileInput.files) void PM.AgentUI?.addAttachments([...fileInput.files]); fileInput.value = ''; }} />
    <button class="agent-round agent-attach" type="button" title={ATTACHMENT_HINT} aria-label="Add attachments" onclick={() => fileInput.click()} disabled={mode.disabled}><Icon {PM} name="plus" /></button>
    <label class="panel-sr-only" for={textareaId}>Message Powermove agent</label>
    <textarea
      id={textareaId}
      rows="1"
      placeholder={mode.placeholder}
      aria-label="Message Powermove agent"
      data-autosize="true"
      disabled={mode.disabled}
      bind:this={textarea}
      value={draft}
      oninput={input}
      onpaste={paste}
      onkeydown={keydown}
    ></textarea>
    {#if mode.working}
      <button class="agent-round agent-stop" type="button" aria-label="Stop current run" title="Stop current run" onclick={() => PM.AgentUI?.stop()}><i aria-hidden="true"></i></button>
    {:else}
      <button
        class="agent-round agent-send"
        class:is-sendable={draft.trim().length > 0 || agentState.attachments.length > 0}
        type="button"
        aria-label={mode.sendLabel}
        title={mode.sendLabel}
        disabled={mode.disabled}
        onclick={submit}
      >
        {#if mode.disabled}<i class="agent-spin" aria-hidden="true"></i>{:else}<Icon {PM} name="return" />{/if}
      </button>
    {/if}
  </div>
</div>
<!-- All run choices share the same quiet options row. -->
<AgentOptions {PM} showPanelFocus={panelId !== 'library'} />
