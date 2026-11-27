<script lang="ts">
  import '@powermove/tokens/tokens.css';
  import '../app.css';
  import { page } from '$app/state';

  let { data, children } = $props();

  const NAV = [
    { href: '/users', label: 'Users' },
    { href: '/extensions', label: 'Extensions' },
    { href: '/aero', label: 'Aero & Billing' },
    { href: '/admins', label: 'Admins' },
    { href: '/log', label: 'Moderation log' },
  ];
  const current = (href: string) => page.url.pathname === href || page.url.pathname.startsWith(`${href}/`);
</script>

{#if data.who.state === 'admin'}
  <div class="shell">
    <aside class="sidebar">
      <nav class="nav" aria-label="Admin">
        <div class="nav-title">Admin</div>
        {#each NAV as item (item.href)}
          <a class="navbtn" href={item.href} aria-current={current(item.href) ? 'page' : undefined}>{item.label}</a>
        {/each}
      </nav>
      <div class="sidebar-foot">
        <div class="who" title={data.who.user.email}>{data.who.user.email}</div>
        <form method="POST" action="/sign-out">
          <button class="navbtn" type="submit">Sign out</button>
        </form>
      </div>
    </aside>
    <main class="sheet">
      <div class="scroll"><div class="column">{@render children()}</div></div>
    </main>
  </div>
{:else}
  <div class="shell bare">
    <main class="sheet">
      <div class="scroll"><div class="column">{@render children()}</div></div>
    </main>
  </div>
{/if}
