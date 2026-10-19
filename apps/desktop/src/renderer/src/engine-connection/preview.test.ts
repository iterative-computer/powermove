import { describe, expect, it } from 'vitest';
import { EnginePreview, type PreviewSession, type PreviewStatus } from './preview';

function rig() {
  const sent: Array<Record<string, unknown>> = [];
  const session: PreviewSession = {
    id: 5, generation: 2, closed: false, route: 'engine',
    post(event) { sent.push(event); return true; },
  };
  let current: PreviewSession | null = session;
  const preview = new EnginePreview(() => current, () => 'comp', () => 0.5, () => 7);
  const status = (phase: PreviewStatus['phase'], generation = Number(sent.at(-1)?.generation), frameSeq: number | null = null, error: string | null = null): PreviewStatus =>
    ({ generation, automatic: false, revision: 7, composition: 'comp', time: 352_800_000, phase, frameSeq, error });
  return { preview, session, sent, status, replace(value: PreviewSession | null) { current = value; } };
}

describe('native paused preview control', () => {
  it('resolves only after the exact engine frame arrives, regardless of event order', async () => {
    const r = rig();
    const completion = r.preview.render();
    expect(r.preview.state()).toEqual({ busy: true, error: null });
    expect(r.sent[0]).toMatchObject({ type: 'previewControl', action: 'render', revision: 7, time: 352_800_000 });
    let done = false;
    void completion.then(() => { done = true; });
    r.preview.receiveStatus(r.session, r.status('ready', 1, 22));
    r.preview.receiveFrame(r.session, { seq: 21, revision: 7, pts: 352_800_000 });
    await Promise.resolve();
    expect(done).toBe(false);
    r.preview.receiveFrame(r.session, { seq: 22, revision: 7, pts: 352_800_000 });
    await completion;
    expect(r.preview.state()).toEqual({ busy: false, error: null });
  });

  it('ignores an old generation after restart and completes the new request', async () => {
    const r = rig();
    const old = r.preview.render();
    const newer = r.preview.render();
    await expect(old).rejects.toMatchObject({ name: 'EnginePreviewCancelled' });
    r.preview.receiveStatus(r.session, r.status('ready', 1, 4));
    expect(r.preview.state().busy).toBe(true);
    r.preview.receiveFrame(r.session, { seq: 5, revision: 7, pts: 352_800_000 });
    r.preview.receiveStatus(r.session, r.status('ready', 2, 5));
    await newer;
  });

  it('cancels native work and ignores a late failed job', async () => {
    const r = rig();
    const rendering = r.preview.render();
    const cancelling = r.preview.cancel();
    await expect(rendering).rejects.toMatchObject({ name: 'EnginePreviewCancelled' });
    r.preview.receiveStatus(r.session, r.status('failed', 1, null, 'old failure'));
    r.preview.receiveStatus(r.session, r.status('cancelled', 2));
    await cancelling;
    expect(r.preview.state()).toEqual({ busy: false, error: null });
    expect(r.sent.at(-1)).toMatchObject({ type: 'previewControl', action: 'cancel' });
  });

  it('reports the engine failure and rejects the matching render', async () => {
    const r = rig();
    const rendering = r.preview.render();
    r.preview.receiveStatus(r.session, r.status('failed', 1, null, 'Blender unavailable'));
    await expect(rendering).rejects.toMatchObject({ name: 'EnginePreviewFailed', message: 'Blender unavailable' });
    expect(r.preview.state()).toEqual({ busy: false, error: 'Blender unavailable' });
  });

  it('fails closed when the native route is not ready', async () => {
    const r = rig();
    r.session.route = 'pending';
    await expect(r.preview.render()).rejects.toMatchObject({ name: 'EnginePreviewUnavailable' });
    expect(r.sent).toHaveLength(0);
  });

  it('prepares the headless cache without waiting for a presented frame', async () => {
    const r = rig();
    r.session.route = 'pending';
    r.session.headless = true;
    const rendering = r.preview.render({ cacheOnly: true, size: { width: 640, height: 360 } });
    await Promise.resolve();
    expect(r.sent.at(-1)).toMatchObject({ action: 'render', cacheOnly: true, size: { width: 640, height: 360 } });
    r.preview.receiveStatus(r.session, { ...r.status('ready', 1), cacheOnly: true });
    await rendering;
    expect(r.preview.state()).toEqual({ busy: false, error: null });
  });

  it('ignores an old cache completion after cancellation', async () => {
    const r = rig();
    r.session.route = 'pending';
    r.session.headless = true;
    const rendering = r.preview.render({ cacheOnly: true, size: { width: 1, height: 1 } });
    await Promise.resolve();
    const cancelling = r.preview.cancel();
    await expect(rendering).rejects.toMatchObject({ name: 'EnginePreviewCancelled' });
    r.preview.receiveStatus(r.session, { ...r.status('ready', 1), cacheOnly: true });
    r.preview.receiveStatus(r.session, r.status('cancelled', 2));
    await cancelling;
    expect(r.preview.state().busy).toBe(false);
  });

  it('cannot start a cache job after cancel wins the session wake', async () => {
    let awake!: () => void;
    const sent: Array<Record<string, unknown>> = [];
    const session: PreviewSession = { id: 3, generation: 1, closed: false, route: 'pending', headless: true, post(event) { sent.push(event); return true; } };
    const preview = new EnginePreview(() => session, () => 'comp', () => 0, () => 1, undefined, () => new Promise<void>(resolve => { awake = resolve; }));
    const rendering = preview.render({ cacheOnly: true, size: { width: 10, height: 10 } });
    await Promise.resolve();
    await Promise.resolve();
    const cancelling = preview.cancel();
    await expect(rendering).rejects.toMatchObject({ name: 'EnginePreviewCancelled' });
    preview.receiveStatus(session, { generation: 1, automatic: false, revision: 1, composition: 'comp', time: 0, phase: 'cancelled', frameSeq: null, error: null });
    await cancelling;
    awake();
    await Promise.resolve();
    expect(sent).toHaveLength(1);
    expect(sent[0]).toMatchObject({ action: 'cancel' });
  });

  it('closes a cache request while its session wake never completes', async () => {
    const r = rig();
    const preview = new EnginePreview(() => r.session, () => 'comp', () => 0, () => 1, undefined, () => new Promise<void>(() => {}));
    const rendering = preview.render({ cacheOnly: true, size: { width: 10, height: 10 } });
    preview.close(r.session);
    await expect(rendering).rejects.toMatchObject({ name: 'EnginePreviewCancelled' });
    expect(r.sent).toHaveLength(0);
  });

  it('disposes both a posted request and a cache wake without waiting for the engine', async () => {
    const r = rig();
    const posted = r.preview.render();
    r.preview.dispose();
    await expect(posted).rejects.toMatchObject({ name: 'EnginePreviewCancelled' });
    await expect(r.preview.render()).rejects.toMatchObject({ name: 'EnginePreviewUnavailable' });
    const preview = new EnginePreview(() => r.session, () => 'comp', () => 0, () => 1, undefined, () => new Promise<void>(() => {}));
    const waking = preview.render({ cacheOnly: true, size: { width: 10, height: 10 } });
    preview.dispose();
    await expect(waking).rejects.toMatchObject({ name: 'EnginePreviewCancelled' });
  });
});
