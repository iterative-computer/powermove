import type { PowermoveAPI } from 'powermove';
import Panel from './Panel.svelte';

/* Test fixture for the sandbox. It registers ordinary contributions and then
   publishes a proof of its own isolation as effect data, because a Playwright
   Electron driver cannot evaluate inside an out-of-process opaque-origin frame. */
export default function activate(api: PowermoveAPI) {
  api.effects.register({ id: `${api.id}.tint`, label: 'Sandbox tint', group: 'Test', params: [], frag: 'o = texture(u_tex, v_uv);' });
  api.commands.register({ id: `${api.id}.command`, label: 'Sandbox command', run: async () => {
    await api.project.apply([]);
    return 'ran';
  } });
  api.status.register({ id: `${api.id}.status`, text: () => `Sandbox ${api.vars.get('TOKEN') ?? 'ready'}` });
  let keyRuns = 0;
  api.commands.register({ id: `${api.id}.key`, label: 'Fixture key', run: () => {
    keyRuns += 1;
    api.effects.register({ id: `${api.id}.key-${keyRuns}`, label: 'Key proof', group: 'Test', params: [], frag: 'o = texture(u_tex, v_uv);' });
  } });
  /* Only the primary fixture binds keys: a second copy of this fixture (the
     e2e installs one without network) would otherwise be refused the chords
     as belonging to another owner, which is the rule working as intended. */
  if (api.id === 'sandboxed-ext') {
    api.keybindings.bind({ key: 'cmd+shift+9', command: `${api.id}.key` });
    api.keybindings.bind({ key: 'alt+shift+f10', command: `${api.id}.key` });
  }
  /* A Svelte panel: in the sandbox it renders in its own view iframe, which
     imports this same bundle and mounts the component by this id. */
  api.panels.register({ id: `${api.id}.panel`, title: 'Sandbox panel', icon: 'puzzle', size: 220, component: Panel });

  void (async () => {
    let deleteRefused = false;
    if (!api.manifest.permissions?.includes('project:write')) {
      try { await api.commands.run('delete'); } catch (error) { deleteRefused = (error as Error).message.includes('project:write'); }
    }
    const powermove = (globalThis as { powermove?: unknown }).powermove !== undefined;
    let parentDenied = false;
    try { void (globalThis as unknown as Window).parent.document; } catch { parentDenied = true; }
    let cspBlocked = false;
    const onViolation = (event: Event) => { if ((event as SecurityPolicyViolationEvent).violatedDirective === 'connect-src') cspBlocked = true; };
    globalThis.addEventListener('securitypolicyviolation', onViolation);
    const abort = new AbortController();
    const timer = setTimeout(() => abort.abort(), 1500);
    await fetch('https://example.com', { mode: 'no-cors', signal: abort.signal }).catch(() => {});
    clearTimeout(timer);
    await new Promise((resolve) => setTimeout(resolve, 50));
    globalThis.removeEventListener('securitypolicyviolation', onViolation);
    api.effects.register({ id: `${api.id}.proof`, label: JSON.stringify({ powermove, parentDenied, cspBlocked, deleteRefused }), group: 'Test', params: [], frag: 'o = texture(u_tex, v_uv);' });
  })();
}
