/*
 * Powermove plans in the renderer: the cached plan (tier, usage, quotas), the
 * upgrade sheet, and share links. The cloud decides every entitlement; this
 * module only asks and shows.
 */
import { flushSync, mount, unmount } from 'svelte';
import type { PlanDto, ProjectShareSnapshot } from '../../../shared/aerolite';
import { cloudBridge, subscribeAccount } from './account';
import UpgradeSheet from './UpgradeSheet.svelte';

type Listener = (plan: PlanDto | null) => void;
let plan: PlanDto | null = null;
let error = '';
let loading: Promise<PlanDto | null> | null = null;
const listeners = new Set<Listener>();
let registry: Record<string, any> | null = null;
let sheet: { close(): void } | null = null;

export function currentPlan(): PlanDto | null { return plan; }
export function planError(): string { return error; }
export function subscribePlan(listener: Listener): () => void {
  listeners.add(listener);
  listener(plan);
  return () => listeners.delete(listener);
}
function set(next: PlanDto | null): void {
  plan = next;
  for (const listener of listeners) listener(plan);
  globalThis.window?.dispatchEvent(new CustomEvent('pm-plan-changed', { detail: plan }));
}

/** Fetches the plan; concurrent callers share one request. */
export function refreshPlan(): Promise<PlanDto | null> {
  const bridge = cloudBridge();
  if (!bridge?.plan) { set(null); return Promise.resolve(null); }
  loading ??= bridge.plan().then(result => {
    error = result.ok ? '' : result.error.detail ?? 'Powermove Cloud is unavailable right now.';
    if (result.ok) set(result.value);
    return result.ok ? result.value : plan;
  }, () => plan).finally(() => { loading = null; });
  return loading;
}

/** Opens Stripe Checkout in the browser. Returns an error message, or '' when the browser opened. */
export async function startCheckout(): Promise<string> {
  const result = await cloudBridge()?.checkout?.({ tier: 'aerolite' });
  if (!result) return 'Plans are available in the desktop app.';
  return result.ok ? '' : result.error.detail ?? 'Couldn’t open checkout. Try again.';
}
export async function openBillingPortal(): Promise<string> {
  const result = await cloudBridge()?.billingPortal?.();
  if (!result) return 'Plans are available in the desktop app.';
  return result.ok ? '' : result.error.detail ?? 'Couldn’t open billing. Try again.';
}

/** The paywall. One at a time. `reason` names what the person was trying to do. */
export function openUpgrade(reason?: string): void {
  const PM = registry as any;
  if (!PM?.modal || sheet) return;
  const body = document.createElement('div');
  let component: ReturnType<typeof mount> | null = null;
  const handle = PM.modal({
    body, width: 420, actions: [],
    onClose: () => { if (component) void unmount(component); component = null; sheet = null; }
  });
  handle.el.classList.add('account-modal', 'upgrade-modal');
  sheet = handle;
  component = mount(UpgradeSheet, { target: body, props: { PM, reason, onclose: () => handle.close() } });
  flushSync();
}

/** Facts about the open project for its read-only share page. Pure, so it is tested without the editor. */
export function projectShareSnapshot(project: Record<string, any>): ProjectShareSnapshot {
  const num = (value: unknown, fallback: number) => typeof value === 'number' && Number.isFinite(value) && value > 0 ? value : fallback;
  const label = (value: unknown, fallback: string) => (typeof value === 'string' && value.trim() ? value.trim() : fallback).slice(0, 120);
  const comps = Object.values(project.comps && typeof project.comps === 'object' ? project.comps : {}) as Array<Record<string, any>>;
  const layers = Array.isArray(project.layers) ? project.layers : [];
  return {
    kind: 'project',
    title: label(project.name, 'Untitled project'),
    width: Math.round(num(project.w, 1920)), height: Math.round(num(project.h, 1080)),
    fps: num(project.fps, 30), durationSeconds: num(project.dur, 0),
    compositions: comps.slice(0, 50).map(comp => ({ name: label(comp.name, 'Composition'), durationSeconds: num(comp.dur, 0), layerCount: Array.isArray(comp.layers) ? comp.layers.length : 0 })),
    layers: layers.slice(0, 300).map((layer: Record<string, any>) => ({
      name: label(layer.name, 'Layer'), type: label(layer.type, 'layer').slice(0, 40),
      ...(typeof layer.from === 'number' && typeof layer.dur === 'number' ? { inSeconds: Math.max(0, layer.from), outSeconds: Math.max(0, layer.from + layer.dur) } : {})
    }))
  };
}

/** A JPEG of the frame on screen, base64, small enough for the 2 MB poster limit. */
async function posterFrame(PM: Record<string, any>): Promise<string | undefined> {
  const canvas: HTMLCanvasElement | undefined = PM.GL?.canvas;
  if (!canvas?.toBlob) return undefined;
  const blob = await new Promise<Blob | null>(resolve => canvas.toBlob(resolve, 'image/jpeg', .85));
  if (!blob) return undefined;
  const bitmap = await createImageBitmap(blob, { resizeWidth: Math.min(1600, canvas.width), resizeHeight: Math.max(1, Math.round(Math.min(1600, canvas.width) * canvas.height / canvas.width)) });
  try {
    const scaled = document.createElement('canvas');
    scaled.width = bitmap.width; scaled.height = bitmap.height;
    scaled.getContext('2d')!.drawImage(bitmap, 0, 0);
    const data = scaled.toDataURL('image/jpeg', .85).split(',')[1];
    return data && data.length < 2_700_000 ? data : undefined;
  } finally { bitmap.close(); }
}

/** Shares the open project as a read-only page. Opens the paywall when the quota is used. */
export async function shareCurrentProject(PM: Record<string, any>): Promise<{ url: string } | { error: string } | null> {
  const bridge = cloudBridge();
  if (!bridge?.createShare) return { error: 'Sharing is available in the desktop app.' };
  if (!PM.proj) return { error: 'Open a project to share it.' };
  const account = PM.Account?.me;
  if (!account) { PM.Account?.signIn?.(); return null; }
  const latest = await refreshPlan();
  if (latest && !latest.shares.canCreate) {
    if (latest.plan.tier === 'free') { openUpgrade('share'); return null; }
    const next = latest.shares.nextAvailableAt ? new Date(latest.shares.nextAvailableAt).toLocaleDateString(undefined, { month: 'short', day: 'numeric' }) : 'next week';
    return { error: `You've used this week's share link. Your next one is available ${next}.` };
  }
  const result = await bridge.createShare({ snapshot: projectShareSnapshot(PM.proj), poster: await posterFrame(PM).catch(() => undefined) });
  void refreshPlan();
  if (!result.ok) return { error: result.error.detail ?? 'Couldn’t create the link. Try again.' };
  await navigator.clipboard?.writeText(result.value.url).catch(() => undefined);
  return { url: result.value.url };
}

export function installPlan(PM: Record<string, any>): void {
  registry = PM;
  PM.Plan = { refresh: refreshPlan, get current() { return plan; }, openUpgrade, shareCurrentProject: () => shareCurrentProject(PM) };
  subscribeAccount(() => { void refreshPlan(); });
  // Returning from Stripe Checkout or the portal in the browser: pick up the new plan.
  globalThis.window?.addEventListener('focus', () => { if (cloudBridge() && PM.Account?.me) void refreshPlan(); });
}
