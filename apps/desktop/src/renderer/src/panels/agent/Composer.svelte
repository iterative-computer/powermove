<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from '../Icon.svelte';
  import AttachmentChips from './AttachmentChips.svelte';
  import { ATTACHMENT_HINT } from './attachments';
  import { agentState, composerMode } from './agent-state.svelte';
  import AgentOptions from './AgentOptions.svelte';
  import SlashMenu from './SlashMenu.svelte';
  import { slashCommands, type SlashCommand } from './slash-commands';

  let { PM, panelId }: { PM: Record<string, any>; panelId: string } = $props();
  let textarea = $state<HTMLTextAreaElement>(null!);
  let fileInput: HTMLInputElement;
  let draft = $state('');
  let dragDepth = $state(0);
  let lastFocusVersion = 0;
  let focused = $state(false);
  let dismissedDraft = $state<string | null>(null);
  let selectedCommand = $state(0);
  const mode = $derived(composerMode(agentState.legacyPhase));
  const textareaId = $derived(`agent-composer-${panelId}`);
  const menuId = $derived(`${textareaId}-commands`);
  const commands = $derived(slashCommands(draft, agentState));
  const showCommands = $derived(focused && !mode.disabled && dismissedDraft !== draft && commands.length > 0);
  const commandIndex = $derived(Math.min(selectedCommand, Math.max(0, commands.length - 1)));

  function setDraft(value: string): void {
    draft = value;
    textarea.value = value;
    PM.AgentUI?.setDraft(value);
    selectedCommand = 0;
    dismissedDraft = null;
    autosize();
  }

  function chooseCommand(command: SlashCommand): void {
    if (!command.value && ['model', 'provider', 'effort'].includes(command.action)) {
      setDraft(`/${command.action} `);
      textarea.focus();
      return;
    }
    setDraft('');
    switch (command.action) {
      case 'new': PM.AgentUI?.newThread(); break;
      case 'model': PM.AgentUI?.setModel(command.value, agentState.reasoningEffort); break;
      case 'provider': PM.AgentUI?.setProvider(command.value); break;
      case 'effort': PM.AgentUI?.setModel(agentState.model, command.value); break;
      case 'attach': fileInput.click(); break;
      case 'stop': PM.AgentUI?.stop(); break;
      case 'settings': PM.SettingsUI?.open('accounts'); return;
    }
    textarea.focus();
  }

  /* Track only the draft and the phase (placeholder copy) — not the snapshot
     revision — so streaming ticks never re-measure the textarea. */
  let sizedPhase = '';
  $effect(() => {
    const next = agentState.composerDraft;
    const nextPhase = agentState.legacyPhase;
    const draftChanged = draft !== next;
    const phaseChanged = sizedPhase !== nextPhase;
    sizedPhase = nextPhase;
    if (draftChanged) draft = next;
    if (textarea && textarea.value !== next) textarea.value = next;
    if (textarea && (draftChanged || phaseChanged)) queueMicrotask(autosize);
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
    setDraft(textarea.value);
  }

  function submit(): void {
    if (showCommands && commands[commandIndex]) { chooseCommand(commands[commandIndex]!); return; }
    if (!textarea.value.trim() && !agentState.attachments.length) return;
    PM.AgentUI?.submit(textarea.value);
    textarea.focus();
  }

  function keydown(event: KeyboardEvent): void {
    // The field-aware keymap dispatches Electron's native clipboard paste.
    if ((event.metaKey || event.ctrlKey) && !event.altKey && !event.shiftKey && event.key.toLowerCase() === 'v') return;
    event.stopPropagation();
    // Enter confirms IME composition; it must never submit an unfinished word.
    if (event.isComposing || event.keyCode === 229) return;
    if (event.key === 'Enter' && event.repeat) { event.preventDefault(); return; }
    if (showCommands) {
      if (event.key === 'Escape') { event.preventDefault(); dismissedDraft = draft; return; }
      if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
        event.preventDefault();
        selectedCommand = (commandIndex + (event.key === 'ArrowDown' ? 1 : -1) + commands.length) % commands.length;
        return;
      }
      if ((event.key === 'Enter' || event.key === 'Tab') && !event.shiftKey) {
        event.preventDefault(); chooseCommand(commands[commandIndex]!); return;
      }
    }
    if (event.key === 'Escape') {
      event.preventDefault();
      textarea.blur();
      return;
    }
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

  onMount(() => {
    queueMicrotask(autosize);
    const owner = textarea.ownerDocument;
    const releaseFocus = (event: PointerEvent) => {
      if (event.button !== 0 && event.button !== 1) return;
      if (owner.activeElement !== textarea) return;
      const composer = textarea.closest('.agent-composer');
      if (composer && event.composedPath().includes(composer)) return;
      // Canvas drags cancel the browser's default focus change. Release the
      // draft before those handlers run so Space returns to editor playback.
      textarea.blur();
    };
    owner.addEventListener('pointerdown', releaseFocus, true);
    return () => owner.removeEventListener('pointerdown', releaseFocus, true);
  });
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
    {#if showCommands && textarea}
      <SlashMenu id={menuId} anchor={textarea} options={commands} selected={commandIndex} choose={chooseCommand} dismiss={() => { dismissedDraft = draft; }} />
    {/if}
    <input class="panel-sr-only" bind:this={fileInput} type="file" multiple onchange={() => { if (fileInput.files) void PM.AgentUI?.addAttachments([...fileInput.files]); fileInput.value = ''; }} />
    <button class="agent-round agent-attach" type="button" title={ATTACHMENT_HINT} aria-label="Add attachments" onclick={() => fileInput.click()} disabled={mode.disabled}><Icon {PM} name="plus" /></button>
    <label class="panel-sr-only" for={textareaId}>Message Powermove agent</label>
    <textarea
      id={textareaId}
      rows="1"
      placeholder={mode.placeholder}
      aria-label="Message Powermove agent"
      aria-controls={showCommands ? menuId : undefined}
      aria-activedescendant={showCommands ? `${menuId}-${commandIndex}` : undefined}
      aria-autocomplete="list"
      title="Enter to send · Shift+Enter for a new line · / for shortcuts"
      data-autosize="true"
      disabled={mode.disabled}
      bind:this={textarea}
      value={draft}
      oninput={input}
      onpaste={paste}
      onkeydown={keydown}
      onfocus={() => { focused = true; }}
      onblur={() => { focused = false; }}
    ></textarea>
    {#if mode.working}
      <button class="agent-round agent-stop" type="button" aria-label="Stop current run" title="Stop current run" onclick={() => PM.AgentUI?.stop()}><i aria-hidden="true"></i></button>
    {/if}
    {#if !mode.working || draft.trim().length > 0 || agentState.attachments.length > 0}
      <button
        class="agent-round agent-send"
        class:is-sendable={draft.trim().length > 0 || agentState.attachments.length > 0}
        type="button"
        aria-label={mode.sendLabel}
        title={mode.sendLabel}
        disabled={mode.disabled || (!draft.trim() && !agentState.attachments.length)}
        onpointerdown={event => event.preventDefault()}
        onclick={submit}
      >
        {#if mode.disabled}<i class="agent-spin" aria-hidden="true"></i>{:else}<Icon {PM} name="return" />{/if}
      </button>
    {/if}
  </div>
  <AgentOptions {PM} />
</div>
