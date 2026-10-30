import { describe, expect, it } from 'vitest';

import { sandboxDocumentUrl, sandboxHost } from '../shared/sandbox-origin';
import { CAD_WORKER_CONTENT_SECURITY_POLICY, CONTENT_SECURITY_POLICY, SANDBOX_CONTENT_SECURITY_POLICY, isCadWorkerScript, extensionSandboxCsp, sandboxFrameNavigationAllowed, sandboxHostOwner, sandboxProcessesToKill } from './security-policy';

describe('renderer security policy', () => {
  it('allows registry images in the editor while keeping generated scripts isolated', () => {
    expect(directive(CONTENT_SECURITY_POLICY, 'img-src')).toEqual(["'self'", 'blob:', 'data:', 'http:', 'https:']);
    expect(SANDBOX_CONTENT_SECURITY_POLICY).toContain("default-src 'none'");
    expect(directive(extensionSandboxCsp('test-icon', [], 'app://test'), 'img-src')).toEqual(['data:', 'blob:']);
  });
  it('allows trusted extensions to make HTTP requests and WebSocket connections', () => {
    const connect = CONTENT_SECURITY_POLICY.split(';').map(value => value.trim())
      .find(value => value.startsWith('connect-src '))?.split(/\s+/).slice(1);
    expect(connect).toEqual(["'self'", 'blob:', 'http:', 'https:', 'ws:', 'wss:']);
    expect(SANDBOX_CONTENT_SECURITY_POLICY).toContain("connect-src 'none'");
  });

  it('confines WebAssembly and eval to the CAD worker, which has no network', () => {
    expect(directive(CAD_WORKER_CONTENT_SECURITY_POLICY, 'script-src')).toEqual(["'self'", "'unsafe-eval'", "'wasm-unsafe-eval'"]);
    expect(directive(CAD_WORKER_CONTENT_SECURITY_POLICY, 'connect-src')).toEqual(["'self'"]);
    expect(CONTENT_SECURITY_POLICY).not.toContain('wasm-unsafe-eval');
    expect(isCadWorkerScript('assets/cad.worker-sP2-XfP1.js')).toBe(true);
    expect(isCadWorkerScript('assets/index-sP2-XfP1.js')).toBe(false);
    expect(isCadWorkerScript('assets/cad.worker-x.js/../index.js')).toBe(false);
  });

  it('forbids eval in the privileged editor and confines it to the generated-script sandbox', () => {
    expect(CONTENT_SECURITY_POLICY).toContain("script-src 'self'");
    expect(CONTENT_SECURITY_POLICY).not.toContain("'unsafe-eval'");
    expect(SANDBOX_CONTENT_SECURITY_POLICY).toContain("'unsafe-eval'");
    expect(SANDBOX_CONTENT_SECURITY_POLICY).toContain("connect-src 'none'");
  });
});

const directive = (policy: string, name: string): string[] | undefined =>
  policy.split(';').map(value => value.trim()).find(value => value.startsWith(`${name} `))?.split(/\s+/).slice(1);

describe('extension sandbox policy', () => {
  const host = sandboxHost('sandboxed-ext');

  it('lets the editor frame app: sandbox hosts', () => {
    expect(directive(CONTENT_SECURITY_POLICY, 'frame-src')).toEqual(["'self'", 'about:', 'blob:', 'app:']);
  });

  it('scopes scripts to the extension’s own origin', () => {
    const policy = extensionSandboxCsp('sandboxed-ext', ['network'], `app://${host}`);
    expect(directive(policy, 'script-src')).toEqual([`app://${host}/host/`, `app://${host}/ext/sandboxed-ext/`, "'wasm-unsafe-eval'"]);
    expect(directive(policy, 'frame-src')).toEqual(["'none'"]);
  });

  it('reads data: and blob: URLs with or without network, and reaches the network only with it', () => {
    const origin = `app://${host}`;
    const network = extensionSandboxCsp('sandboxed-ext', ['network'], origin);
    const offline = extensionSandboxCsp('sandboxed-ext', [], origin);
    expect(directive(network, 'connect-src')).toEqual(['data:', 'blob:', 'https:', 'wss:']);
    expect(directive(offline, 'connect-src')).toEqual(['data:', 'blob:']);
    expect(directive(offline, 'connect-src')).not.toContain("'none'");
    expect(directive(network, 'media-src')).toEqual(['data:', 'blob:', 'https:']);
    expect(directive(offline, 'media-src')).toEqual(['data:', 'blob:']);
    // Never plain http or ws, even with network.
    for (const policy of [network, offline]) expect(policy).not.toMatch(/(?:^|\s)(?:http|ws):/);
  });

  it('loads bundled fonts always, and remote fonts and stylesheets only with network', () => {
    const origin = `app://${host}`;
    const network = extensionSandboxCsp('sandboxed-ext', ['network'], origin);
    const offline = extensionSandboxCsp('sandboxed-ext', [], origin);
    expect(directive(network, 'font-src')).toEqual([`${origin}/`, 'data:', 'blob:', 'https:']);
    expect(directive(offline, 'font-src')).toEqual([`${origin}/`, 'data:', 'blob:']);
    expect(directive(network, 'style-src')).toEqual(["'unsafe-inline'", `${origin}/`, 'https:']);
    expect(directive(offline, 'style-src')).toEqual(["'unsafe-inline'", `${origin}/`]);
    // Scripts never widen with network.
    expect(directive(network, 'script-src')).toEqual(directive(offline, 'script-src'));
  });

  it('adds the dev server to connect-src without dropping data: and blob:', () => {
    const dev = 'http://localhost:5173';
    expect(directive(extensionSandboxCsp('sandboxed-ext', [], `app://${host}`, dev), 'connect-src')).toEqual(['data:', 'blob:', dev, 'ws://localhost:5173']);
    expect(directive(extensionSandboxCsp('sandboxed-ext', ['network'], `app://${host}`, dev), 'connect-src')).toEqual(['data:', 'blob:', 'https:', 'wss:', dev, 'ws://localhost:5173']);
  });
});

describe('sandbox frame navigation', () => {
  const own = sandboxDocumentUrl(`app://${sandboxHost('a-ext')}`, 'a-ext', '');
  const other = sandboxDocumentUrl(`app://${sandboxHost('b-ext')}`, 'b-ext', '');

  it('allows a sandbox document on its own host from the editor', () => {
    expect(sandboxFrameNavigationAllowed(own, ['', 'app://powermove/'])).toBe(true);
    expect(sandboxFrameNavigationAllowed(own, [undefined, null])).toBe(true);
    expect(sandboxFrameNavigationAllowed(own, [own, 'app://powermove/'])).toBe(true);
  });

  it('refuses mismatched hosts, other paths and one sandbox becoming another', () => {
    expect(sandboxFrameNavigationAllowed(own.replace('id=a-ext', 'id=b-ext'), [''])).toBe(false);
    expect(sandboxFrameNavigationAllowed(`app://${sandboxHost('a-ext')}/index.html`, [''])).toBe(false);
    expect(sandboxFrameNavigationAllowed('app://powermove/host/ext-sandbox.html?id=a-ext&perms=', [''])).toBe(false);
    expect(sandboxFrameNavigationAllowed('https://example.com/', [''])).toBe(false);
    expect(sandboxFrameNavigationAllowed(other, [own])).toBe(false);
    expect(sandboxFrameNavigationAllowed(other, ['', own])).toBe(false);
  });
});

describe('sandboxProcessesToKill', () => {
  const a = sandboxHost('a-ext'), b = sandboxHost('b-ext');
  const url = (host: string) => `app://${host}/host/ext-sandbox.html?id=x`;
  const editor = (pid: number, frames: Array<{ url: string; pid: number }>) => ({ mainFramePid: pid, frames: [{ url: 'app://powermove/', pid }, ...frames] });

  it('kills a process that only holds that extension’s frames, across windows', () => {
    const contents = [
      editor(10, [{ url: url(a), pid: 20 }, { url: url(a), pid: 20 }, { url: url(b), pid: 21 }]),
      editor(11, [{ url: url(a), pid: 20 }, { url: url(a), pid: 22 }])
    ];
    expect(sandboxProcessesToKill(a, contents).sort()).toEqual([20, 22]);
    expect(sandboxProcessesToKill(b, contents)).toEqual([21]);
  });

  it('spares a process shared with another host or with any main frame', () => {
    expect(sandboxProcessesToKill(a, [editor(10, [{ url: url(a), pid: 20 }, { url: url(b), pid: 20 }])])).toEqual([]);
    expect(sandboxProcessesToKill(a, [editor(10, [{ url: url(a), pid: 10 }])])).toEqual([]);
    expect(sandboxProcessesToKill(a, [editor(10, [{ url: url(a), pid: 20 }]), { mainFramePid: 20, frames: [] }])).toEqual([]);
    expect(sandboxProcessesToKill(a, [editor(10, [{ url: url(a), pid: 20 }, { url: 'about:blank', pid: 20 }])])).toEqual([]);
  });

  it('ignores unknown pids and non-sandbox hosts', () => {
    expect(sandboxProcessesToKill(a, [editor(10, [{ url: url(a), pid: 0 }, { url: url(a), pid: -1 }])])).toEqual([]);
    expect(sandboxProcessesToKill('powermove', [editor(10, [{ url: 'app://powermove/x', pid: 20 }])])).toEqual([]);
  });
});

describe('sandboxHostOwner', () => {
  const a = sandboxHost('a-ext');

  it('is the one installed id on that host', () => {
    expect(sandboxHostOwner(a, ['b-ext', 'a-ext', 'a-ext'])).toBe('a-ext');
    expect(sandboxHostOwner(a, ['b-ext'])).toBeNull();
    expect(sandboxHostOwner(a, [])).toBeNull();
    expect(sandboxHostOwner('powermove', ['a-ext'])).toBeNull();
  });

  it('fails closed when two ids share a host', () => {
    const colliding = (id: string) => (id === 'victim' || id === 'crafted' ? a : sandboxHost(id));
    expect(sandboxHostOwner(a, ['victim', 'crafted'], colliding)).toBeNull();
    expect(sandboxHostOwner(a, ['victim', 'other'], colliding)).toBe('victim');
  });
});
