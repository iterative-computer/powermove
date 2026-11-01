<script lang="ts">
  import { blocksFromMarkdown, inlineRuns, prose, wordsFromRuns, type Block, type Media, type Run } from './markdown';
  import { revealText } from './text-reveal';
  import { bridge } from '../../kernel/bridge';

  /* Agent prose as real elements: headings, paragraphs, nested and task
     lists, quotes, fenced code, tables, rules, and the images, video and
     sound a reply names, with bold / italic / code / links inline. Words are
     spans so the reveal can stagger while streaming; only the deepest last
     paragraph carries the caret. Blocks re-parse on every chunk and keep
     their keys, so an existing word never remounts as the stream extends. */

  let {
    text,
    streaming = false,
    animated = false,
    paragraphClass = '',
    inline = false,
    links = true
  }: { text: string; streaming?: boolean; animated?: boolean; paragraphClass?: string; inline?: boolean; links?: boolean } = $props();

  const blocks = $derived(blocksFromMarkdown(text));

  let expanded = $state<string | null>(null);
  let failed = $state<Record<string, true>>({});
  /** Web images load only after the user asks: a crafted image URL could carry data out. */
  let allowed = $state<Record<string, true>>({});
  const visible = (runs: Run[]): boolean => runs.some(run => run.text.trim());
  const fail = (item: Media): void => { failed = { ...failed, [item.path]: true }; };
  const host = (url: string): string => { try { return new URL(url).host; } catch { return url; } };

  function openLink(event: MouseEvent, href: string): void {
    event.preventDefault();
    void bridge()?.openExternal?.(href);
  }
</script>

{#snippet words(runs: Run[], live: boolean)}{#each wordsFromRuns(runs) as word (word.key)}{#if word.href && links}<a href={word.href} class="agent-md-link" class:is-bold={word.b} class:is-italic={word.i} class:is-struck={word.s} onclick={(event) => openLink(event, word.href!)}>{#if word.c}<code class="agent-trace-code">{word.text}</code>{:else}{word.text}{/if}</a>{:else if word.c}<code class="agent-trace-code">{word.text}</code>{:else}<span use:revealText={animated && live && Boolean(word.text.trim())} class:is-bold={word.b} class:is-italic={word.i} class:is-struck={word.s}>{word.text}</span>{/if}{/each}{/snippet}
{#snippet gallery(media: Media[] | undefined)}{#if media?.length}<div class="agent-md-media">
    {#each media as item (item.path)}
      {#if failed[item.path]}
        <p class="agent-md-media-missing">Couldn't show {item.path}</p>
      {:else if item.remote && !allowed[item.path]}
        <button type="button" class="agent-md-media-load" title={item.path} onclick={() => allowed = { ...allowed, [item.path]: true }}>
          Show {item.kind} from {host(item.path)}
        </button>
      {:else if item.kind === 'image'}
        <button type="button" class="agent-md-media-frame" class:is-expanded={expanded === item.path} title={item.path}
          aria-label={expanded === item.path ? `Shrink ${item.alt}` : `Enlarge ${item.alt}`}
          onclick={() => expanded = expanded === item.path ? null : item.path}>
          <img src={item.src} alt={item.alt} loading="lazy" draggable="false" referrerpolicy="no-referrer" onerror={() => fail(item)} />
        </button>
      {:else if item.kind === 'video'}
        <!-- svelte-ignore a11y_media_has_caption -->
        <video class="agent-md-media-frame" src={item.src} title={item.path} controls loop playsinline preload="metadata" onerror={() => fail(item)}></video>
      {:else}
        <audio class="agent-md-audio" src={item.src} title={item.path} controls preload="metadata" onerror={() => fail(item)}></audio>
      {/if}
    {/each}
  </div>{/if}{/snippet}
{#snippet flow(list: Block[], live: boolean, depth: number)}{#each list as block, bi (bi)}
  {@const on = live && bi === list.length - 1}
  {#if block.kind === 'p'}
    {#if visible(block.runs)}<p class="agent-md-p {paragraphClass}" class:is-streaming={on && !block.media}>{@render words(block.runs, on)}</p>{/if}{@render gallery(block.media)}
  {:else if block.kind === 'h'}
    <p class="agent-md-h {paragraphClass}" class:is-streaming={on && !block.media} data-level={block.level} role="heading" aria-level={block.level}>{@render words(block.runs, on)}</p>{@render gallery(block.media)}
  {:else if block.kind === 'list'}
    <svelte:element this={block.ordered ? 'ol' : 'ul'} class="agent-md-list" start={block.ordered ? block.start : undefined} data-depth={depth}>
      {#each block.items as item, ii (ii)}
        <li class="agent-md-li" class:is-task={item.checked !== undefined}>
          {#if item.checked !== undefined}
            <span class="agent-md-check" class:is-checked={item.checked} role="img" aria-label={item.checked ? 'Done' : 'Not done'}></span>
          {:else if block.ordered}
            <span class="agent-md-ordinal">{block.start + ii}.</span>
          {:else}
            <i class="agent-md-bullet" aria-hidden="true"></i>
          {/if}
          <div class="agent-md-li-body" class:is-done={item.checked}>{@render flow(item.blocks, on && ii === block.items.length - 1, depth + 1)}</div>
        </li>
      {/each}
    </svelte:element>
  {:else if block.kind === 'quote'}
    <blockquote class="agent-md-quote">{@render flow(block.blocks, on, depth)}</blockquote>
  {:else if block.kind === 'hr'}
    <hr class="agent-md-rule" />
  {:else if block.kind === 'table'}
    <div class="agent-md-table-wrap"><table>
      <thead><tr>{#each block.headers as cell, ci}<th style:text-align={block.align[ci]}>{@render words(cell, false)}</th>{/each}</tr></thead>
      <tbody>{#each block.rows as row}<tr>{#each row as cell, ci}<td style:text-align={block.align[ci]}>{@render words(cell, false)}</td>{/each}</tr>{/each}</tbody>
    </table></div>
  {:else}
    <pre class="agent-md-pre" data-lang={block.lang}><code>{block.text}</code></pre>
  {/if}
{/each}{/snippet}
{#if inline}{@render words(prose(inlineRuns(text)).runs, false)}{:else}{@render flow(blocks, streaming, 0)}{/if}

<style>
  .agent-trace-code { border-radius: var(--r-xs); padding: 0 4px; background: var(--ink-1); font-family: var(--f-mono); font-size: .92em; box-decoration-break: clone; }
  :is(.agent-md-p, .agent-md-h, .agent-md-list, .agent-md-quote, .agent-md-pre, .agent-md-media, .agent-md-table-wrap) + :is(.agent-md-p, .agent-md-h, .agent-md-list, .agent-md-quote, .agent-md-pre, .agent-md-media) { margin-top: .65em; }
  .agent-md-p, .agent-md-h { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; }
  .agent-md-h { font-weight: var(--fw-semibold); color: var(--tx); }
  .agent-md-h[data-level="1"], .agent-md-h[data-level="2"] { font-size: 1.08em; }
  .is-bold { font-weight: var(--fw-semibold); }
  .is-struck { text-decoration: line-through; }
  .agent-md-rule { border: 0; border-top: 1px solid var(--line-2); margin: .8em 0; }
  .agent-md-table-wrap { max-width: 100%; overflow-x: auto; margin: .65em 0; }
  table { width: 100%; border-collapse: collapse; font: inherit; }
  th, td { padding: 6px 8px; border-bottom: 1px solid var(--line-2); text-align: left; overflow-wrap: anywhere; }
  th { font-weight: var(--fw-semibold); }
  .is-italic { font-style: italic; }
  .agent-md-list { margin: 0; padding: 0; list-style: none; }
  .agent-md-li { display: flex; gap: 8px; min-width: 0; }
  .agent-md-li + .agent-md-li { margin-top: .2em; }
  .agent-md-li .agent-md-list { margin-top: .2em; }
  .agent-md-li-body { flex: 1; min-width: 0; }
  .agent-md-li-body.is-done { color: var(--tx-3); }
  .agent-md-ordinal { flex: none; min-width: 1.1em; color: var(--tx-3); text-align: right; font-variant-numeric: tabular-nums; }
  .agent-md-bullet { flex: none; width: 5px; height: 5px; margin-top: .62em; border-radius: 50%; background: var(--tx-3); }
  .agent-md-list[data-depth="1"] > li > .agent-md-bullet { background: transparent; box-shadow: inset 0 0 0 1px var(--tx-3); }
  .agent-md-list:not([data-depth="0"]):not([data-depth="1"]) > li > .agent-md-bullet { border-radius: 1px; }
  .agent-md-check {
    flex: none; position: relative; width: 12px; height: 12px; margin-top: .3em; border-radius: 3px;
    box-shadow: inset 0 0 0 1px var(--tx-3);
  }
  .agent-md-check.is-checked { background: var(--accent); box-shadow: none; }
  .agent-md-check.is-checked::after {
    content: ""; position: absolute; left: 4px; top: 1.5px; width: 3px; height: 6px;
    border: solid #fff; border-width: 0 1.5px 1.5px 0; transform: rotate(45deg);
  }
  .agent-md-quote { margin: 0; padding-left: 10px; border-left: 2px solid var(--line-2); color: var(--tx-2); }
  .agent-md-pre {
    margin: 0; padding: 8px 10px; border-radius: var(--r-md);
    background: var(--ink-1); box-shadow: inset 0 0 0 var(--hairline) var(--line-2);
    color: var(--tx); font: var(--fs-sm)/1.55 var(--f-mono); white-space: pre-wrap; overflow-wrap: anywhere;
  }
  .agent-md-link { color: var(--accent-tx); text-decoration: underline; text-decoration-color: color-mix(in srgb, currentColor 35%, transparent); text-underline-offset: 2px; transition: text-decoration-color var(--dur-1); }
  .agent-md-link:hover { text-decoration-color: currentColor; }
  .agent-md-media { display: flex; flex-direction: column; align-items: flex-start; gap: 8px; }
  .agent-md-media-frame {
    display: block; max-width: 100%; padding: 0; border: 0; border-radius: var(--r-md); overflow: hidden;
    background: var(--ink-1); box-shadow: inset 0 0 0 var(--hairline) var(--line-2); cursor: zoom-in;
  }
  .agent-md-media-frame.is-expanded { cursor: zoom-out; }
  .agent-md-media-frame img, video.agent-md-media-frame { display: block; max-width: 100%; max-height: 320px; object-fit: contain; }
  video.agent-md-media-frame { cursor: default; }
  .agent-md-media-frame.is-expanded img { max-height: none; }
  .agent-md-media-frame:focus-visible, .agent-md-media-load:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .agent-md-media-load {
    padding: 8px 12px; border: 0; border-radius: var(--r-md); background: var(--ink-1);
    box-shadow: inset 0 0 0 var(--hairline) var(--line-2); color: var(--tx-2); font: inherit; font-size: var(--fs-sm); cursor: pointer;
  }
  .agent-md-media-load:hover { color: var(--tx); }
  .agent-md-audio { width: 100%; max-width: 360px; }
  .agent-md-media-missing { margin: 0; color: var(--tx-3); font-size: var(--fs-sm); overflow-wrap: anywhere; }
</style>
