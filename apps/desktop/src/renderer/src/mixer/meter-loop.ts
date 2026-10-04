/*
 * The meter draw loop. One requestAnimationFrame loop for the whole panel:
 * read every tap, step the ballistics, paint each strip's canvas. It runs only
 * while the panel is visible and either audio is playing or a meter is still
 * falling; then it releases the taps and stops, so an idle mixer costs nothing.
 *
 * Nothing here reads layout. Canvas sizes arrive from ResizeObserver through
 * `resize`, colours from `setPalette` when the theme changes.
 */
import {
  createChannelMeter, meterPosition, meterRmsDb, meterSettled, METER_HOT_DB, METER_WARN_DB,
  resetChannelMeter, stepChannelMeter, type ChannelMeter
} from './levels';

export interface LevelSource {
  readonly MASTER: string;
  read(id: string, out: { peak: number[]; rms: number[] }): boolean;
  release(): void;
  running(): boolean;
}

export interface MeterPalette {
  track: string;
  nominal: string;
  warn: string;
  hot: string;
}

interface MeterView {
  id: string;
  canvas: HTMLCanvasElement;
  context: CanvasRenderingContext2D | null;
  width: number;
  height: number;
  ratio: number;
  channels: [ChannelMeter, ChannelMeter];
  clipped: boolean;
  onClip: (clipped: boolean) => void;
  gradient: CanvasGradient | null;
  drawn: string;
}

type Schedule = (callback: (now: number) => void) => number;

export class MeterLoop {
  private views = new Map<string, MeterView>();
  private visible = false;
  private frame = 0;
  private last = 0;
  private palette: MeterPalette = { track: 'rgba(128,128,128,.12)', nominal: '#3FCF8E', warn: '#F59E0B', hot: '#F36357' };
  private scratch = { peak: [0, 0], rms: [0, 0] };
  /** Called once per drawn frame while audio runs (faders following automation). */
  onFrame: (() => void) | null = null;

  constructor(
    private readonly source: LevelSource,
    private readonly schedule: Schedule = (callback) => requestAnimationFrame(callback),
    private readonly cancel: (handle: number) => void = (handle) => cancelAnimationFrame(handle)
  ) {}

  attach(id: string, canvas: HTMLCanvasElement, onClip: (clipped: boolean) => void): () => void {
    const view: MeterView = {
      id, canvas, context: canvas.getContext('2d'), width: 0, height: 0, ratio: 1,
      channels: [createChannelMeter(), createChannelMeter()], clipped: false, onClip, gradient: null, drawn: ''
    };
    this.views.set(id, view);
    this.kick();
    return () => {
      if (this.views.get(id) === view) this.views.delete(id);
    };
  }

  resize(id: string, width: number, height: number, ratio: number): void {
    const view = this.views.get(id);
    if (!view) return;
    view.width = Math.max(0, width);
    view.height = Math.max(0, height);
    view.ratio = Math.max(1, ratio);
    view.canvas.width = Math.round(view.width * view.ratio);
    view.canvas.height = Math.round(view.height * view.ratio);
    view.gradient = null;
    view.drawn = '';
    this.draw(view);
  }

  setPalette(palette: MeterPalette): void {
    this.palette = palette;
    for (const view of this.views.values()) { view.gradient = null; view.drawn = ''; this.draw(view); }
  }

  setVisible(visible: boolean): void {
    if (this.visible === visible) return;
    this.visible = visible;
    if (visible) this.kick();
    else this.stop();
  }

  /** Start drawing if there is anything to draw. Safe to call often. */
  kick(): void {
    if (!this.visible || this.frame) return;
    this.last = 0;
    this.frame = this.schedule(this.tick);
  }

  clearClip(id: string): void {
    const view = this.views.get(id);
    if (!view) return;
    for (const channel of view.channels) channel.clipped = false;
    this.setClipped(view, false);
  }

  /** Clear every clip indicator (playback restarted from a stop). */
  clearClips(): void {
    for (const id of this.views.keys()) this.clearClip(id);
  }

  reset(): void {
    for (const view of this.views.values()) {
      for (const channel of view.channels) resetChannelMeter(channel);
      this.setClipped(view, false);
      view.drawn = '';
      this.draw(view);
    }
  }

  dispose(): void {
    this.stop();
    this.views.clear();
  }

  get running(): boolean {
    return this.frame !== 0;
  }

  private stop(): void {
    if (this.frame) this.cancel(this.frame);
    this.frame = 0;
    this.source.release();
  }

  private tick = (now: number): void => {
    this.frame = 0;
    if (!this.visible) { this.source.release(); return; }
    const dt = this.last ? (now - this.last) / 1000 : 1 / 60;
    this.last = now;
    const playing = this.source.running();
    let moving = false;
    for (const view of this.views.values()) {
      const read = playing && this.source.read(view.id, this.scratch);
      for (let channel = 0; channel < 2; channel++) {
        stepChannelMeter(view.channels[channel]!, read ? this.scratch.peak[channel]! : 0, read ? this.scratch.rms[channel]! : 0, dt);
      }
      this.draw(view);
      const clipped = view.channels[0].clipped || view.channels[1].clipped;
      if (clipped !== view.clipped) this.setClipped(view, clipped);
      if (!meterSettled(view.channels[0]) || !meterSettled(view.channels[1])) moving = true;
    }
    if (playing) this.onFrame?.();
    if (playing || moving) this.frame = this.schedule(this.tick);
    else this.source.release();
  };

  private setClipped(view: MeterView, clipped: boolean): void {
    view.clipped = clipped;
    view.onClip(clipped);
  }

  private zoneGradient(view: MeterView, context: CanvasRenderingContext2D): CanvasGradient {
    if (view.gradient) return view.gradient;
    const height = view.canvas.height;
    const gradient = context.createLinearGradient(0, height, 0, 0);
    const warn = meterPosition(METER_WARN_DB);
    const hot = meterPosition(METER_HOT_DB);
    gradient.addColorStop(0, this.palette.nominal);
    gradient.addColorStop(warn, this.palette.nominal);
    gradient.addColorStop(warn, this.palette.warn);
    gradient.addColorStop(hot, this.palette.warn);
    gradient.addColorStop(hot, this.palette.hot);
    gradient.addColorStop(1, this.palette.hot);
    view.gradient = gradient;
    return gradient;
  }

  private draw(view: MeterView): void {
    const context = view.context;
    const width = view.canvas.width;
    const height = view.canvas.height;
    if (!context || width < 2 || height < 2) return;
    const [left, right] = view.channels;
    // Quantise to device pixels; an unchanged meter is not repainted.
    const rows = (db: number) => Math.round(meterPosition(db) * height);
    const levels = [left, right].map((channel) => [rows(meterRmsDb(channel)), rows(channel.peak), rows(channel.hold)]);
    const signature = levels.flat().join(',');
    if (signature === view.drawn) return;
    view.drawn = signature;

    const gap = Math.max(1, Math.round(view.ratio * 2));
    const bar = Math.floor((width - gap) / 2);
    const line = Math.max(1, Math.round(view.ratio));
    const gradient = this.zoneGradient(view, context);
    context.clearRect(0, 0, width, height);
    for (let channel = 0; channel < 2; channel++) {
      const x = channel === 0 ? 0 : width - bar;
      const [rms, peak, hold] = levels[channel]!;
      context.globalAlpha = 1;
      context.fillStyle = this.palette.track;
      context.fillRect(x, 0, bar, height);
      context.fillStyle = gradient;
      if (peak! > rms!) {
        // The instantaneous peak reads as a lighter extension of the RMS body.
        context.globalAlpha = 0.42;
        context.fillRect(x, height - peak!, bar, peak! - rms!);
      }
      context.globalAlpha = 1;
      if (rms! > 0) context.fillRect(x, height - rms!, bar, rms!);
      if (hold! > 0) context.fillRect(x, Math.max(0, height - hold!), bar, line);
    }
    context.globalAlpha = 1;
  }
}
