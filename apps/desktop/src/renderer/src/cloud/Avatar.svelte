<script lang="ts">
  import { initials, type CloudUser } from './account';

  let { user, size = 20 }: { user: CloudUser; size?: number } = $props();

  /* Small avatars carry one letter; two would crowd the circle. */
  const letters = $derived(size < 24 ? initials(user.name).slice(0, 1) : initials(user.name));
</script>

<span
  class="acct-avatar"
  aria-hidden="true"
  style:--size={`${size}px`}
>{letters}</span>
