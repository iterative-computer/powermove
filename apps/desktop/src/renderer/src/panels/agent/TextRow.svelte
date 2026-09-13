<script lang="ts">
  import Markdown from './Markdown.svelte';

  /* Model prose in the activity trail, rendered as markdown. While the stream
     is still landing the last block ends in a solid caret (a pseudo-element,
     so every real child stays a plain inline word span); once it settles the
     caret is gone. Words fade in only while streaming, so a replayed
     transcript paints in one go. */

  let { text, streaming = false, animated = false }: { text: string; streaming?: boolean; animated?: boolean } = $props();
</script>

<div class="agent-trace-prose"><Markdown {text} {streaming} {animated} paragraphClass="agent-trace-text" /></div>
