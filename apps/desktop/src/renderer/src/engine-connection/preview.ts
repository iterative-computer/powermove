import { ticks } from './transport';

export type PreviewAction = 'render' | 'cancel';
export type PreviewPhase = 'queued' | 'running' | 'ready' | 'cancelled' | 'failed';
export type PreviewStatus = {
  generation: number; automatic: boolean; revision: number; composition: string; time: number;
  phase: PreviewPhase; frameSeq: number | null; error: string | null;
};
export type PreviewFrame = { seq: number; revision: number; pts: number };
export type PreviewSession = {
  id: number | null; generation: number; closed: boolean; route: string;
  post(event: Record<string, unknown> & { type: string }): boolean;
};
export type PreviewState = { busy: boolean; error: string | null };

type Pending = {
  session: PreviewSession; status: PreviewStatus | null;
  resolve(): void; reject(error: Error): void;
};

/** One explicit renderFrame/cancelRender operation in the interactive engine.
 * A Ready status alone is insufficient: the matching frame must reach the
 * viewer as well, including when GPU completion sends it after the status. */
export class EnginePreview {
  private generation = 0;
  private pending: Pending | null = null;
  private cancelPending: Pending | null = null;
  private frames = new Map<number, PreviewFrame>();
  private currentSession: PreviewSession | null = null;
  private autoGeneration = 0;
  private value: PreviewState = { busy: false, error: null };
  private listeners = new Set<(state: PreviewState) => void>();

  constructor(
    private readonly session: () => PreviewSession | null,
    private readonly composition: () => string,
    private readonly time: () => number,
    private readonly revision: () => number,
    private readonly shading: () => 'material' | 'rendered' = () => 'rendered',
  ) {}

  state(): PreviewState { return { ...this.value }; }
  subscribe(listener: (state: PreviewState) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  private emit(busy: boolean, error: string | null): void {
    this.value = { busy, error };
    for (const listener of this.listeners) listener(this.state());
  }

  private validSession(): PreviewSession {
    const session = this.session();
    if (!session || session.closed || session.id === null || session.route !== 'engine') {
      const error = new Error('The native viewer is not ready to render this composition');
      error.name = 'EnginePreviewUnavailable';
      throw error;
    }
    if (this.currentSession !== session) {
      this.abort('The native viewer session restarted');
      this.frames.clear();
      this.autoGeneration = 0;
      this.currentSession = session;
    }
    return session;
  }

  private abort(message: string): void {
    const error = new Error(message);
    error.name = 'EnginePreviewCancelled';
    this.pending?.reject(error);
    this.cancelPending?.reject(error);
    this.pending = null;
    this.cancelPending = null;
    this.emit(false, null);
  }

  render(): Promise<void> {
    let session: PreviewSession;
    try { session = this.validSession(); } catch (error) { return Promise.reject(error); }
    this.abort('The preview was replaced by a newer render');
    const control = {
      type: 'previewControl', generation: ++this.generation,
      revision: Math.max(0, Math.trunc(this.revision())),
      composition: this.composition(), time: ticks(this.time()), shading: this.shading(), action: 'render' as const,
    };
    if (!control.composition || !session.post(control)) {
      const error = new Error('The native viewer could not accept the preview request');
      error.name = 'EnginePreviewUnavailable';
      return Promise.reject(error);
    }
    this.emit(true, null);
    return new Promise<void>((resolve, reject) => {
      this.pending = { session, status: { ...control, automatic: false, phase: 'queued', frameSeq: null, error: null }, resolve, reject };
    });
  }

  cancel(): Promise<void> {
    let session: PreviewSession;
    try { session = this.validSession(); } catch (error) { return Promise.reject(error); }
    this.abort('The preview was cancelled');
    const control = {
      type: 'previewControl', generation: ++this.generation,
      revision: Math.max(0, Math.trunc(this.revision())),
      composition: this.composition(), time: ticks(this.time()), shading: this.shading(), action: 'cancel' as const,
    };
    if (!control.composition || !session.post(control)) {
      const error = new Error('The native viewer could not cancel the preview');
      error.name = 'EnginePreviewUnavailable';
      return Promise.reject(error);
    }
    return new Promise<void>((resolve, reject) => {
      this.cancelPending = { session, status: { ...control, automatic: false, phase: 'queued', frameSeq: null, error: null }, resolve, reject };
    });
  }

  receiveStatus(session: PreviewSession, status: PreviewStatus): void {
    if (status.automatic) {
      if (this.session() !== session || session.closed || this.pending) return;
      if (this.currentSession !== session) {
        this.currentSession = session;
        this.frames.clear();
        this.autoGeneration = 0;
      }
      if (status.generation < this.autoGeneration) return;
      this.autoGeneration = status.generation;
      if (status.phase === 'queued' || status.phase === 'running') this.emit(true, null);
      else if (status.phase === 'failed') this.emit(false, status.error || 'The native preview failed');
      else this.emit(false, null);
      return;
    }
    const pending = status.phase === 'cancelled' && this.cancelPending?.status?.generation === status.generation
      ? this.cancelPending : this.pending;
    if (!pending || pending.session !== session || status.generation !== pending.status?.generation) return;
    if (status.revision !== pending.status.revision || status.composition !== pending.status.composition || status.time !== pending.status.time) return;
    if (status.phase === 'failed') {
      this.pending = null;
      const error = new Error(status.error || 'The native preview failed');
      error.name = 'EnginePreviewFailed';
      this.emit(false, error.message);
      pending.reject(error);
    } else if (status.phase === 'cancelled') {
      const wasCancel = pending === this.cancelPending;
      if (wasCancel) this.cancelPending = null;
      else this.pending = null;
      this.emit(false, null);
      if (wasCancel) pending.resolve();
      else {
        const error = new Error('The native preview was cancelled');
        error.name = 'EnginePreviewCancelled';
        pending.reject(error);
      }
    } else if (status.phase === 'ready') {
      pending.status = status;
      this.finishIfPresented();
    } else if (status.phase === 'queued' || status.phase === 'running') {
      pending.status = status;
      this.emit(true, null);
    }
  }

  receiveFrame(session: PreviewSession, frame: PreviewFrame): void {
    if (this.session() !== session || session.closed) return;
    if (this.currentSession !== session) {
      this.currentSession = session;
      this.frames.clear();
    }
    this.frames.set(frame.seq, frame);
    while (this.frames.size > 8) this.frames.delete(this.frames.keys().next().value!);
    this.finishIfPresented();
  }

  private finishIfPresented(): void {
    const pending = this.pending, status = pending?.status;
    if (!pending || status?.phase !== 'ready' || status.frameSeq === null) return;
    const frame = this.frames.get(status.frameSeq);
    if (!frame || frame.revision < status.revision || frame.pts !== status.time) return;
    this.pending = null;
    this.emit(false, null);
    pending.resolve();
  }

  close(session: PreviewSession): void {
    if (this.currentSession !== session) return;
    this.abort('The native viewer session closed');
    this.currentSession = null;
    this.autoGeneration = 0;
    this.frames.clear();
  }
}
