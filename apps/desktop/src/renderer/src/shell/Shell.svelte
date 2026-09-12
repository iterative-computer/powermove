<script lang="ts">
  import StatusBar from './StatusBar.svelte';
  import Titlebar from './Titlebar.svelte';

  let { PM, part }: {
    PM: Record<string, any>;
    part: 'titlebar' | 'status';
  } = $props();
</script>

<!-- The static #app remains the layout owner. installShell mounts one fragment
     into each existing chrome target and deliberately never mounts over #body. -->
{#if part === 'titlebar'}
  <Titlebar {PM} />
{:else}
  <StatusBar {PM} />
{/if}
