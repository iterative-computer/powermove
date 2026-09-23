<script lang="ts">
  import { onMount, tick, untrack } from 'svelte';
  import { fade, fly } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import Icon from '../panels/Icon.svelte';
  import { ALL } from '../store/fixtures';
  import iconLight from './icon-light.png';
  import iconDark from './icon-dark.png';
  import { PROVIDER_LABEL, type AccountProvider, type CloudUser, type SignInMode } from './account';

  let { PM, mode: startMode, onsignedin, oncancel }: {
    PM: Record<string, any>;
    mode: SignInMode;
    onsignedin: (user: CloudUser) => void;
    oncancel: () => void;
  } = $props();

  /* One sheet, four steps. Start offers the providers and email; a provider
     hands off to the browser, email asks for a code, and a new account ends
     by claiming a handle, the publisher namespace on the Store. Design pass:
     every server round trip is a timer. */
  type Step = 'start' | 'browser' | 'code' | 'handle';
  type HandleState = 'empty' | 'invalid' | 'checking' | 'available' | 'taken';

  const PROVIDERS: AccountProvider[] = ['apple', 'google', 'github'];
  const CODE_LENGTH = 6;
  const RESEND_AFTER = 30;
  const HANDLE = /^[a-z0-9][a-z0-9-]{1,38}$/;
  const TAKEN = new Set([...ALL.map((l) => l.publisher), 'powermove', 'admin', 'store', 'support', 'team']);
  /* Who a provider says you are, until there is a provider to ask. */
  const PROVIDER_IDENTITY = { name: 'Jude Kim', handle: 'jude', email: 'jude@trypowermove.com' };
  const TERMS = 'https://trypowermove.com/terms';
  const PRIVACY = 'https://trypowermove.com/privacy';

  const reduced = () => matchMedia('(prefers-reduced-motion: reduce)').matches;

  let mode = $state<SignInMode>(untrack(() => startMode));
  let step = $state<Step>('start');
  let direction = $state(0);
  let provider = $state<AccountProvider>('email');
  let email = $state('');
  let code = $state('');
  let codeFocused = $state(false);
  let verifying = $state(false);
  let wrongCode = $state(false);
  let resendIn = $state(0);
  let name = $state('');
  let handle = $state('');
  let handleState = $state<HandleState>('empty');
  let creating = $state(false);
  let root: HTMLElement;
  let codeInput = $state<HTMLInputElement | null>(null);
  let timer = 0;

  const emailValid = $derived(/^[^\s@]+@[^\s@]+\.[^\s@]{2,}$/.test(email.trim()));
  const providerLabel = $derived(PROVIDER_LABEL[provider]);
  const canCreate = $derived(handleState === 'available' && name.trim().length > 0 && !creating);
  /* Pushing a step slides in from the right, going back from the left, as
     the Store's detail pages do. The first step just appears. */
  const slide = $derived(reduced() || direction === 0 ? { duration: 0 } : { x: direction * 16, duration: 200, easing: cubicOut });
  const handleCopy = $derived({
    empty: 'Letters, numbers and hyphens.',
    invalid: 'At least 2 characters, starting with a letter or number.',
    checking: 'Checking…',
    available: `@${handle} is available.`,
    taken: `@${handle} is taken.`
  }[handleState]);

  function focusStep(): void {
    void tick().then(() => (root?.querySelector<HTMLElement>('[data-autofocus]') ?? root)?.focus({ preventScroll: true }));
  }

  function go(next: Step, towards: number): void {
    window.clearTimeout(timer);
    direction = towards;
    step = next;
    focusStep();
  }

  function back(): void {
    verifying = false;
    creating = false;
    go('start', -1);
  }

  function finish(user: CloudUser): void {
    onsignedin(user);
    PM.toast?.(`Signed in as ${user.name}`, 2200, { kind: 'status', icon: 'userCircle' });
  }

  /* ── providers: the browser does the asking ── */
  function viaProvider(next: AccountProvider): void {
    provider = next;
    go('browser', 1);
    timer = window.setTimeout(approved, 2400);
  }

  function reopen(): void {
    window.clearTimeout(timer);
    timer = window.setTimeout(approved, 1600);
  }

  function approved(): void {
    if (mode === 'sign-up') {
      name = PROVIDER_IDENTITY.name;
      handle = PROVIDER_IDENTITY.handle;
      go('handle', 1);
      return;
    }
    finish({ ...PROVIDER_IDENTITY, provider });
  }

  /* ── email: a six-digit code ── */
  function sendCode(event: SubmitEvent): void {
    event.preventDefault();
    if (!emailValid) return;
    email = email.trim();
    provider = 'email';
    code = '';
    wrongCode = false;
    resendIn = RESEND_AFTER;
    go('code', 1);
  }

  function resend(): void {
    code = '';
    wrongCode = false;
    resendIn = RESEND_AFTER;
    codeInput?.focus();
  }

  function typeCode(event: Event): void {
    const input = event.currentTarget as HTMLInputElement;
    code = input.value.replace(/\D/g, '').slice(0, CODE_LENGTH);
    input.value = code;
    wrongCode = false;
    if (code.length === CODE_LENGTH) verify();
  }

  /* Any code passes except 000000, so the wrong-code state can be seen. */
  function verify(): void {
    verifying = true;
    codeInput?.blur();
    timer = window.setTimeout(() => {
      verifying = false;
      if (code === '000000') {
        wrongCode = true;
        void tick().then(() => { codeInput?.focus(); codeInput?.select(); });
        return;
      }
      const local = email.split('@')[0] ?? '';
      if (mode === 'sign-up') {
        name = local.split(/[._-]+/).filter(Boolean).map((w) => w[0]!.toUpperCase() + w.slice(1)).join(' ');
        handle = local.toLowerCase().replace(/[^a-z0-9-]/g, '').slice(0, 39);
        go('handle', 1);
        return;
      }
      finish({ name: PROVIDER_IDENTITY.name, handle: PROVIDER_IDENTITY.handle, email, provider: 'email' });
    }, 700);
  }

  /* ── handle: the publisher namespace ── */
  function typeHandle(event: Event): void {
    const input = event.currentTarget as HTMLInputElement;
    handle = input.value.toLowerCase().replace(/[^a-z0-9-]/g, '').slice(0, 39);
    input.value = handle;
  }

  function create(event: SubmitEvent): void {
    event.preventDefault();
    if (!canCreate) return;
    creating = true;
    timer = window.setTimeout(() => {
      finish({ name: name.trim(), handle, email: provider === 'email' ? email : PROVIDER_IDENTITY.email, provider });
    }, 800);
  }

  function external(url: string): void {
    const bridge = (window as any).powermove;
    if (bridge?.openExternal) void bridge.openExternal(url);
    else window.open(url, '_blank', 'noopener');
  }

  $effect(() => {
    const value = handle;
    if (step !== 'handle') return;
    if (!value) { handleState = 'empty'; return; }
    if (!HANDLE.test(value)) { handleState = 'invalid'; return; }
    handleState = 'checking';
    const check = window.setTimeout(() => { handleState = TAKEN.has(value) ? 'taken' : 'available'; }, 360);
    return () => window.clearTimeout(check);
  });

  $effect(() => {
    if (step !== 'code' || resendIn <= 0) return;
    const tickDown = window.setTimeout(() => { resendIn -= 1; }, 1000);
    return () => window.clearTimeout(tickDown);
  });

  onMount(() => () => window.clearTimeout(timer));
</script>

<!-- The sheet itself takes focus on open, so nothing is lit before you act.
     Later steps move focus to their one field. -->
<!-- svelte-ignore a11y_autofocus -->
<div class="acct-sheet" bind:this={root} tabindex="-1" autofocus>
  {#key step}
    <div class="acct-step" in:fly={slide}>
      {#if step === 'start'}
        <img class="acct-mark is-light" src={iconLight} alt="" width="48" height="48" draggable="false" />
        <img class="acct-mark is-dark" src={iconDark} alt="" width="48" height="48" draggable="false" />
        {#key mode}
          <header class="acct-head" in:fade={{ duration: reduced() ? 0 : 120 }}>
            <h2>{mode === 'sign-in' ? 'Sign in to Powermove' : 'Create your Powermove account'}</h2>
            <p>{mode === 'sign-in'
              ? 'Publish and manage your extensions on the Store.'
              : 'Everything you publish to the Store lives under your handle.'}</p>
          </header>
        {/key}
        <div class="acct-providers">
          {#each PROVIDERS as p (p)}
            <button class="btn acct-provider" type="button" data-provider={p} onclick={() => viaProvider(p)}>
              <Icon {PM} name={p} />
              <span>Continue with {PROVIDER_LABEL[p]}</span>
            </button>
          {/each}
        </div>
        <form class="acct-email" novalidate onsubmit={sendCode}>
          <input
            class="acct-field"
            type="email"
            placeholder="Email address"
            aria-label="Email address"
            autocomplete="email"
            spellcheck="false"
            bind:value={email}
          />
          <button class="btn pri acct-submit" type="submit" disabled={!emailValid}>Continue with email</button>
        </form>
        <footer class="acct-foot">
          <p>
            {mode === 'sign-in' ? 'New to Powermove?' : 'Already have an account?'}
            <button class="acct-link" type="button" onclick={() => (mode = mode === 'sign-in' ? 'sign-up' : 'sign-in')}>
              {mode === 'sign-in' ? 'Create an account' : 'Sign in'}
            </button>
          </p>
          <button class="btn ghost" type="button" onclick={oncancel}>Cancel</button>
        </footer>

      {:else if step === 'browser'}
        <button class="acct-back" type="button" onclick={back}><Icon {PM} name="chev" /><span>Back</span></button>
        <header class="acct-head">
          <h2>Continue in your browser</h2>
          <p>Powermove opened {providerLabel} in your browser. Approve the sign-in there and you’ll come straight back here.</p>
        </header>
        <div class="acct-wait" role="status">
          <svg class="acct-spinner" viewBox="0 0 16 16" aria-hidden="true">
            {#each { length: 12 } as _, i}
              <line x1="8" y1="1.75" x2="8" y2="4.5" transform={`rotate(${i * 30} 8 8)`} opacity={0.16 + 0.84 * ((i + 1) / 12)} />
            {/each}
          </svg>
          <span>Waiting for {providerLabel}…</span>
          <button class="btn ghost" type="button" data-autofocus onclick={reopen}>Open again</button>
        </div>

      {:else if step === 'code'}
        <button class="acct-back" type="button" onclick={back}><Icon {PM} name="chev" /><span>Back</span></button>
        <header class="acct-head">
          <h2>Check your email</h2>
          <p>Enter the 6-digit code sent to <b>{email}</b>.</p>
        </header>
        <label class="acct-code" class:wrong={wrongCode} class:verifying>
          <input
            bind:this={codeInput}
            type="text"
            inputmode="numeric"
            autocomplete="one-time-code"
            maxlength={CODE_LENGTH}
            aria-label="Verification code"
            aria-invalid={wrongCode}
            data-autofocus
            disabled={verifying}
            value={code}
            oninput={typeCode}
            onfocus={() => (codeFocused = true)}
            onblur={() => (codeFocused = false)}
          />
          {#each { length: CODE_LENGTH } as _, i}
            <span
              class="acct-cell"
              class:filled={i < code.length}
              class:active={codeFocused && i === Math.min(code.length, CODE_LENGTH - 1)}
              aria-hidden="true"
            >{code[i] ?? ''}</span>
          {/each}
        </label>
        <div class="acct-code-foot">
          <span class="acct-status" class:is-wrong={wrongCode} role="status">
            {#if verifying}Checking the code…{:else if wrongCode}That code didn’t match.{/if}
          </span>
          <button class="btn ghost" type="button" disabled={resendIn > 0 || verifying} onclick={resend}>
            {resendIn > 0 ? `Resend in ${resendIn}s` : 'Resend code'}
          </button>
        </div>

      {:else}
        <button class="acct-back" type="button" onclick={back}><Icon {PM} name="chev" /><span>Back</span></button>
        <header class="acct-head">
          <h2>Choose your handle</h2>
          <p>It’s your name on the Store. Extensions you publish live under it.</p>
        </header>
        <form class="sg-column acct-form" novalidate onsubmit={create}>
          <div class="sg-group">
            <label class="settings-row">
              <span class="settings-copy"><b>Name</b></span>
              <input class="settings-input" type="text" placeholder="Your name" autocomplete="name" spellcheck="false" bind:value={name} />
            </label>
            <label class="settings-row">
              <span class="settings-copy">
                <b>Handle</b>
                <span class="acct-handle-status" data-state={handleState} role="status">{handleCopy}</span>
              </span>
              <span class="acct-handle-field">
                <span aria-hidden="true">@</span>
                <input
                  type="text"
                  aria-label="Handle"
                  autocomplete="username"
                  autocapitalize="off"
                  spellcheck="false"
                  maxlength="39"
                  data-autofocus
                  value={handle}
                  oninput={typeHandle}
                />
              </span>
            </label>
          </div>
          <p class="acct-coord">Your extensions publish as <code>{handle || 'handle'}/my-extension</code></p>
          <button class="btn pri acct-submit" type="submit" disabled={!canCreate}>
            {creating ? 'Creating account…' : 'Create account'}
          </button>
          <p class="acct-legal">
            By creating an account, you agree to the
            <button class="acct-link" type="button" onclick={() => external(TERMS)}>Terms</button>
            and
            <button class="acct-link" type="button" onclick={() => external(PRIVACY)}>Privacy Policy</button>.
          </p>
        </form>
      {/if}
    </div>
  {/key}
</div>
