<script lang="ts">
  import { doc } from '../state/document.svelte';
  import { transport } from '../state/transport.svelte';
  import { EditGesture, type EditBinding } from './gesture';
  import { rowLabelId } from './context';
  import './controls.css';

  let {
    PM,
    get,
    edit,
    label,
    align = 'right',
    mono = true
  }: {
    PM: Record<string, any>;
    get: () => unknown;
    edit: EditBinding;
    label?: string;
    align?: 'left' | 'center' | 'right';
    mono?: boolean;
  } = $props();

  const labelledBy = rowLabelId();
  const value = $derived((doc.tick.values, doc.proj, transport.time, get()));
  const gesture = $derived(new EditGesture(PM, edit));
  let input: HTMLInputElement;
  let live = false;
  let draft = $state('');

  $effect(() => {
    const next = value == null ? '' : String(value);
    if (!live) draft = next;
  });

  function focus(): void {
    gesture.begin();
    live = true;
  }

  function inputValue(): void {
    draft = input.value;
    gesture.write(draft);
    PM.invalidate?.('render');
  }

  function blur(): void {
    if (!live) return;
    gesture.commit();
    live = false;
  }

  function keydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === 'Enter') input.blur();
    if (event.key === 'Escape' && live) {
      event.preventDefault();
      gesture.cancel();
      live = false;
      draft = value == null ? '' : String(value);
      input.blur();
    }
  }
</script>

<input
  bind:this={input}
  class="pm-control-input"
  value={draft}
  aria-labelledby={labelledBy}
  aria-label={labelledBy ? undefined : (label ?? edit.label)}
  style:text-align={align}
  style:font-family={mono ? 'var(--f-mono)' : 'var(--f-ui)'}
  style:font-size="var(--fs-md)"
  style:width="100%"
  style:min-width="40px"
  onfocus={focus}
  oninput={inputValue}
  onblur={blur}
  onkeydown={keydown}
/>

