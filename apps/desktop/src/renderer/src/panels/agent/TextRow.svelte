<script lang="ts">
  import { toRichWords } from './rich-words';
  import { revealText } from './text-reveal';

  /* Model prose in the activity trail. While the stream is still landing the
     paragraph ends in a solid caret (a pseudo-element, so every real child
     stays a plain inline word span); once it settles the caret is gone. Words
     fade in only while streaming, so a replayed transcript paints in one go. */

  let { text, streaming = false, animated = false }: { text: string; streaming?: boolean; animated?: boolean } = $props();
</script>

<p class="agent-trace-text" class:is-streaming={streaming}>{#each toRichWords(text) as word, wi (wi)}<span use:revealText={animated && streaming && Boolean(word.w.trim())} class={word.c ? 'agent-trace-code' : ''}>{word.w}</span>{/each}</p>
