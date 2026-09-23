<script lang="ts">
  import Icon from '../panels/Icon.svelte';
  import Avatar from './Avatar.svelte';
  import { openAccountMenu, openSignIn, subscribeAccount, type CloudUser } from './account';
  import type { PopoverMenuHandle } from '../controls/popover-menu';

  let { PM }: { PM: Record<string, any> } = $props();

  let user = $state<CloudUser | null>(null);
  let open = $state(false);
  let button: HTMLButtonElement;
  let menu: PopoverMenuHandle | null = null;

  $effect(() => subscribeAccount((next) => { user = next; }));

  /* Signed in, the row is your profile and opens the account menu above it,
     staying pressed while the menu is up; pressing again closes it. Signed
     out, it opens the sheet. */
  function press(): void {
    if (!user) return openSignIn();
    if (menu) return menu.close();
    menu = openAccountMenu(button, () => { open = false; menu = null; });
    open = !!menu;
  }
</script>

<button
  bind:this={button}
  class="ps-navbtn acct-row"
  class:open
  class:signed-in={!!user}
  type="button"
  aria-haspopup={user ? 'menu' : 'dialog'}
  aria-expanded={user ? open : undefined}
  aria-label={user ? `Account: ${user.name}` : undefined}
  onclick={press}
>
  {#if user}
    <Avatar {user} size={20} />
    <span>{user.name}</span>
  {:else}
    <Icon {PM} name="userCircle" />
    <span>Sign in</span>
  {/if}
</button>
