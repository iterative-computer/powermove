<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from '../Icon.svelte';
  import AccessPicker from './AccessPicker.svelte';
  import AttachmentChips from './AttachmentChips.svelte';
  import { agentState, composerMode } from './agent-state.svelte';
  import ModelPicker from './ModelPicker.svelte';

  let { PM, panelId }: { PM: Record<string, any>; panelId: string } = $props();
  let textarea: HTMLTextAreaElement;
  let fileInput: HTMLInputElement;
  let draft = $state('');
  let lastFocusVersion = 0;
  const mode = $derived(composerMode(agentState.legacyPhase));
  const textareaId = $derived(`agent-composer-${panelId}`);

  const visiblePanels = $derived.by(() => {
    agentState.revision;
    const workspace = PM.WS?.current;
    const visible = new Set((workspace?.layout?.docks || []).flatMap((dock: any) => (dock.panels || []).map((panel: any) => panel.id)));
    const hidden = new Set((workspace?.hiddenPanels || []).map((item: any) => item.id));
    return Object.entries(PM.PANELS || {}).filter(([id]) => id !== 'toolbar' && (visible.has(id) || hidden.has(id))).map(([id, panel]: [string, any]) => ({
      id,
      title: id === 'viewer' ? 'Composition panel' : panel.title,
      hidden: hidden.has(id)
    }));
  });

  $effect(() => {
    agentState.revision;
    const next = agentState.composerDraft;
    if (draft !== next) draft = next;
    if (textarea && textarea.value !== next) textarea.value = next;
    if (textarea) queueMicrotask(autosize);
  });

  $effect(() => {
    const panels = visiblePanels;
    const scope = agentState.scope;
    if (scope.startsWith('panel:') && !panels.some((panel) => `panel:${panel.id}` === scope)) {
      queueMicrotask(() => {
        if (agentState.scope === scope) PM.AgentUI?.setScope('workspace');
      });
    }
  });

  $effect(() => {
    const next = agentState.focusVersion;
    if (next !== lastFocusVersion && !mode.disabled) {
      lastFocusVersion = next;
      queueMicrotask(() => textarea?.focus());
    }
  });

  function autosize(): void {
    if (!textarea || textarea.closest('[data-popout-source]')) return;
    textarea.style.height = 'auto';
    const style = window.getComputedStyle(textarea);
    const layout = PM.SpatialAssistant.math.textareaLayout(
      textarea.scrollHeight,
      Number.parseFloat(style.minHeight),
      Number.parseFloat(style.maxHeight)
    );
    textarea.style.height = `${layout.height}px`;
    textarea.style.overflowY = layout.overflowY;
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

  function scopeChange(event: Event): void {
    PM.AgentUI?.setScope((event.currentTarget as HTMLSelectElement).value);
  }

  onMount(() => queueMicrotask(autosize));
</script>

<div class="agent-composer">
  <div class="agent-composer-head">
    <label class="agent-scope" title="Choose what the agent should work on">
      <Icon {PM} name="panel" />
      <select aria-label="Agent scope" value={agentState.scope} onchange={scopeChange} onkeydown={(event) => { event.stopPropagation(); if (event.key === 'Escape') (event.currentTarget as HTMLSelectElement).blur(); }}>
        <option value="workspace">Entire workspace</option>
        <option value="composition">Composition</option>
        <optgroup label="Panel">
          {#each visiblePanels as panel (panel.id)}
            <option value={`panel:${panel.id}`}>{panel.title}{panel.hidden ? ' · hidden' : ''}</option>
          {/each}
        </optgroup>
      </select>
      <Icon {PM} name="chev" />
    </label>
    <AccessPicker {PM} />
    {#if agentState.accessMode === 'editor'}
      <button
        class="agent-approval"
        type="button"
        aria-pressed={agentState.autoApplyPanels}
        title={agentState.autoApplyPanels ? 'Safe panel changes apply automatically' : 'Review panel changes before applying'}
        onclick={() => PM.AgentUI?.toggleAutoApplyPanels()}
      ><i aria-hidden="true"></i>{agentState.autoApplyPanels ? 'Auto-apply panels' : 'Review panel edits'}</button>
    {/if}
  </div>
  <label class="panel-sr-only" for={textareaId}>Message Powermove agent</label>
  <textarea
    id={textareaId}
    class="spatial-followup"
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
  <div class="agent-attachment-rail">
    <AttachmentChips {PM} items={agentState.attachments} removable onRemove={(id) => PM.AgentUI?.removeAttachment(id)} />
  </div>
  <div class="agent-composer-tools">
    <input class="panel-sr-only" bind:this={fileInput} type="file" multiple onchange={() => { if (fileInput.files) void PM.AgentUI?.addAttachments([...fileInput.files]); fileInput.value = ''; }} />
    <button class="agent-attach" type="button" title="Attach images or text files" aria-label="Add attachments" onclick={() => fileInput.click()} disabled={mode.disabled}><Icon {PM} name="plus" /></button>
    <span class="sp"></span>
    <ModelPicker {PM} />
    {#if mode.working}
      <button class="agent-stop" type="button" aria-label="Stop current run" title="Stop current run" onclick={() => PM.AgentUI?.stop()}><Icon {PM} name="x" /></button>
    {/if}
    <button class="spatial-action pri agent-send" type="button" aria-label={mode.sendLabel} title={mode.sendLabel} disabled={mode.disabled} onclick={submit}>
      {#if mode.disabled}<i aria-hidden="true"></i>{:else}<Icon {PM} name="return" />{/if}
    </button>
  </div>
</div>

<style>
  .agent-composer {
    position: relative;
  }
</style>
