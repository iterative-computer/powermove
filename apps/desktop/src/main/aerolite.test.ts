import { afterEach, describe, expect, it, vi } from 'vitest';
import { mkdtemp, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
vi.mock('electron', () => ({ safeStorage: { isEncryptionAvailable: () => true, encryptString: (v: string) => Buffer.from(v), decryptString: (v: Buffer) => v.toString() } }));
import { ApiError } from '@powermove/registry/wire';
import { CompatibleProvider } from './compatible-provider';
import { createAerolite } from './cloud/aerolite';
import { cloudSchemas } from './cloud/ipc';

const dirs: string[] = [];
afterEach(async () => { for (const dir of dirs.splice(0)) await rm(dir, { recursive: true, force: true }); });
const sse = (text: string) => new Response(`data: ${JSON.stringify({ choices: [{ delta: { content: text }, finish_reason: 'stop' }] })}\n\ndata: [DONE]\n\n`);
const request = (model: string) => ({ id: 'run-1', provider: 'aerolite', model, mode: 'editor', access: 'editor', prompt: 'Say hi', schema: {}, images: [], attachments: [] }) as any;

describe('Aerolite hosted runner', () => {
  it('sends runs to the Powermove proxy with the session token, never a saved key', async () => {
    const dir = await mkdtemp(path.join(os.tmpdir(), 'pm-aerolite-')); dirs.push(dir);
    const calls: Array<{ url: string; headers: Record<string, string>; body: any }> = [];
    const fetcher = vi.fn(async (url: any, init: any) => { calls.push({ url: String(url), headers: init.headers, body: JSON.parse(init.body) }); return sse('{"operation":"noop","message":"hi"}'); }) as typeof fetch;
    const provider = new CompatibleProvider(dir, fetcher, async () => ({ baseUrl: 'http://localhost:8787/v1/ai', key: 'session-token', models: ['claude-sonnet-5-5', 'gpt-6.2-sol'], vision: true }));
    const result = await provider.run(request('gpt-6.2-sol'), () => {});
    expect(result.ok, JSON.stringify(result)).toBe(true);
    expect(calls[0]!.url).toBe('http://localhost:8787/v1/ai/chat/completions');
    expect(calls[0]!.headers.Authorization).toBe('Bearer session-token');
    expect(calls[0]!.body.model).toBe('gpt-6.2-sol');
  });
  it('refuses models the plan does not include and shows the cloud’s own quota message', async () => {
    const dir = await mkdtemp(path.join(os.tmpdir(), 'pm-aerolite-')); dirs.push(dir);
    const hosted = async () => ({ baseUrl: 'https://cloud.example/v1/ai', key: 't', models: ['claude-sonnet-5-5'], vision: false });
    let provider = new CompatibleProvider(dir, vi.fn() as any, hosted);
    expect((await provider.run(request('claude-opus-5-5'), () => {})).ok).toBe(false);
    provider = new CompatibleProvider(dir, (async () => Response.json({ error: 'quota_exceeded', detail: 'You’ve used today’s Claude allowance. It refills tomorrow.' }, { status: 429 })) as any, hosted);
    const result = await provider.run(request('claude-sonnet-5-5'), () => {});
    expect(result).toMatchObject({ ok: false, error: 'You’ve used today’s Claude allowance. It refills tomorrow.' });
  });
  it('explains what to do when the account cannot use hosted models', async () => {
    const dir = await mkdtemp(path.join(os.tmpdir(), 'pm-aerolite-')); dirs.push(dir);
    const provider = new CompatibleProvider(dir, vi.fn() as any, async () => { throw new Error('Built-in models are part of Aerolite. Upgrade in Settings › Plan & Usage.'); });
    expect(await provider.run(request('claude-sonnet-5-5'), () => {})).toMatchObject({ ok: false, error: expect.stringContaining('Aerolite') });
  });
});

describe('Aerolite cloud client', () => {
  const session = (token: string | null, response: () => Response) => ({
    currentToken: () => token,
    client: () => ({ origin: 'https://cloud.example', raw: async (fn: any) => { const r = await fn(null, async () => response()); if (!r.ok) throw new ApiError(await r.json()); return r; } }),
  }) as any;
  it('opens only Stripe checkout URLs', async () => {
    const opened: string[] = [];
    const good = createAerolite(session('t', () => Response.json({ url: 'https://checkout.stripe.com/c/pay/cs_test_1' })), async url => { opened.push(url); });
    await good.checkout('aerolite');
    expect(opened).toEqual(['https://checkout.stripe.com/c/pay/cs_test_1']);
    const evil = createAerolite(session('t', () => Response.json({ url: 'https://stripe.com.evil.example/pay' })), async url => { opened.push(url); });
    await expect(evil.checkout('aerolite')).rejects.toThrow('unexpected address');
    expect(opened).toHaveLength(1);
  });
  it('is signed out without a token and hands the runner the proxy origin', async () => {
    expect(await createAerolite(session(null, () => new Response()), async () => {}).plan()).toBeNull();
    await expect(createAerolite(session(null, () => new Response()), async () => {}).hostedConnection()).rejects.toThrow('Sign in');
    const plan = { plan: { tier: 'aerolite', label: 'Aerolite', priceCents: 900, storageBytes: 1, shares: { kind: 'weekly', count: 1 }, interactiveShares: false, models: [{ id: 'claude-sonnet-5-5', provider: 'anthropic', label: 'Claude Sonnet 5.5' }] },
      subscription: null, usage: null, shares: { used: 0, usedThisWeek: 0, canCreate: true, nextAvailableAt: null }, storage: { usedBytes: 0, limitBytes: 1 }, upgrades: [], billingConfigured: true };
    const hosted = await createAerolite(session('tok', () => Response.json(plan)), async () => {}).hostedConnection();
    expect(hosted).toEqual({ baseUrl: 'https://cloud.example/v1/ai', key: 'tok', models: ['claude-sonnet-5-5'], vision: true });
  });
  it('validates renderer payloads for plan channels', () => {
    expect(cloudSchemas['cloud:checkout'].safeParse({ tier: 'platinum' }).success).toBe(false);
    expect(cloudSchemas['cloud:share-revoke'].safeParse({ id: '../../v1/me' }).success).toBe(false);
    expect(cloudSchemas['cloud:share-create'].safeParse({ snapshot: { kind: 'project', title: 'x', blob: 'y'.repeat(300_000) } }).success).toBe(false);
    expect(cloudSchemas['cloud:share-create'].safeParse({ snapshot: { kind: 'project', title: 'Launch' } }).success).toBe(true);
  });
});
