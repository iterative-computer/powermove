import type { AffineMatrix, Point, PowermoveAPI, TextCaretLayout } from 'powermove';

import { caretAt, indexAtPoint, paragraphRangeAt, selectionRects, textFrame, verticalMove, wordRangeAt, type Rect } from './text-geometry';

/* On-canvas text editing.

   The compositor keeps painting the layer while it is edited, so what the
   user sees is exactly what renders and exports: same font loading, same
   wrapping, same metrics, no DOM re-implementation of the text. A hidden
   textarea owns keyboard input, IME composition, clipboard and native
   caret movement; every change is written to the project inside one undo
   transaction. The overlay draws the frame, caret and selection from the
   rasterizer's own line geometry.

   Behavior contract (Figma / Penpot conventions):
   - Text tool click creates point text; drag creates fixed-width paragraph
     text with automatic height. Both enter editing at once.
   - Escape, Command+Enter and clicking outside all COMMIT. Nothing typed is
     ever discarded by leaving; Undo reverts the whole session in one step.
   - Finishing with empty text removes the layer (a fresh layer leaves no
     history entry at all).
   - After editing, the tool returns to Select.
   - Clicking inside the edited text moves the caret; dragging selects;
     double-click selects a word; triple-click selects a paragraph.
   - Command+A selects all, Command+B / I toggle weight and italic,
     Command+Shift+> / < step the size (Option: by ten), Command+Option+L /
     T / R align. Enter with one text layer selected starts editing it. */

export interface TextEditorHost {
  inner: HTMLElement;
  shown: number;
  canvasTextEditing?: string | null;
  finishCanvasText?: (() => void) | null;
  textSelection?: { layer: string; start: number; end: number } | null;
  requestOverlay?(): void;
}

export interface TextEditOptions {
  /** The caller already opened the transaction and created the layer in it. */
  fresh?: boolean;
  /** Composition-space point to place the caret at (Text tool click). */
  caretAt?: Point;
  /** Select all text on entry (double-click with the Select tool). */
  selectAll?: boolean;
  drag?(event: PointerEvent, options: { move(dx: number, dy: number, event: PointerEvent): void; up?(): void; cancel?(): void; cursor?: string }): void;
  /** Called once when the session ends, however it ended. */
  onFinish?(): void;
}

export interface TextEditSession {
  readonly layerId: string;
  /** Composition-space hit test against the edited text's frame. */
  contains(point: Point): boolean;
  /** Handle a stage pointerdown; returns true when the session consumed it. */
  pointerDown(event: PointerEvent, point: Point): boolean;
  draw(c: CanvasRenderingContext2D, S: number, ink: string): void;
  finish(): void;
  readonly element: HTMLTextAreaElement;
}

const CARET_BLINK_MS = 530;
const HIT_SLACK = 6;

function applyMatrix(m: AffineMatrix, point: Point): Point {
  return { x: m[0] * point.x + m[2] * point.y + m[4], y: m[1] * point.x + m[3] * point.y + m[5] };
}

function invertPoint(m: AffineMatrix, point: Point): Point | null {
  const det = m[0] * m[3] - m[1] * m[2];
  if (Math.abs(det) < 1e-9) return null;
  const dx = point.x - m[4], dy = point.y - m[5];
  return { x: (dx * m[3] - dy * m[2]) / det, y: (dy * m[0] - dx * m[1]) / det };
}

function firstLine(text: string): string {
  const line = text.split('\n').find(part => part.trim().length) ?? '';
  const trimmed = line.trim();
  return trimmed.length > 40 ? trimmed.slice(0, 39).trimEnd() + '…' : trimmed;
}

export function beginTextEdit(api: PowermoveAPI, V: TextEditorHost, layer: any, options: TextEditOptions = {}): TextEditSession | null {
  if (!layer || layer.lock) return null;
  V.finishCanvasText?.();
  api.selection.select([layer.id]);
  api.transport.pause();
  if (!options.fresh) {
    // A transaction left open elsewhere would otherwise throw here and block
    // every structural edit in the app; close it rather than inherit it.
    try { api.edit.begin('Edit text', { origin: 'canvas' }); }
    catch { api.edit.commit(); api.edit.begin('Edit text', { origin: 'canvas' }); }
  }

  const id: string = layer.id;
  const current = () => api.model.layer(id) ?? layer;
  const time = () => api.transport.time();
  const content = () => api.anim.resolveContent(current(), time()) as any;
  const sourceText = () => String(content()?.text ?? '');
  const layout = (): TextCaretLayout | null => api.render.textLayout(current(), time());
  /* Font anchoring shifts the painted glyphs; the caret follows that shift. */
  const matrix = (): AffineMatrix => {
    const m = [...api.anim.worldMatrix(current(), time())] as AffineMatrix;
    const offset = api.render.raster(current(), 1, time())?.fontOffset as { x?: number; y?: number } | undefined;
    if (offset) { const x = offset.x ?? 0, y = offset.y ?? 0; m[4] += m[0] * x + m[2] * y; m[5] += m[1] * x + m[3] * y; }
    return m;
  };

  const el = document.createElement('textarea');
  el.className = 'canvas-text-input';
  el.setAttribute('aria-label', 'Edit text on canvas');
  el.setAttribute('aria-multiline', 'true');
  el.autocomplete = 'off'; el.spellcheck = false; el.wrap = 'off';
  el.setAttribute('autocapitalize', 'off');
  el.value = sourceText();
  /* The field sits over the text and receives pointer events so a right
     click opens the native text services menu (Look Up, Cut, Copy, Paste).
     Left clicks reach the stage first in its capture phase and are routed to
     pointerDown below, so caret placement stays canvas-driven. */
  el.addEventListener('pointerdown', (e) => { if (e.button === 0) e.preventDefault(); });
  Object.assign(el.style, {
    position: 'absolute', left: '0', top: '0', width: '1px', height: '1px', margin: '0', padding: '0', border: '0',
    resize: 'none', overflow: 'hidden', outline: 'none', background: 'transparent', color: 'transparent',
    caretColor: 'transparent', whiteSpace: 'pre', zIndex: '8', opacity: '0.01', cursor: 'text',
  });
  V.inner.appendChild(el);
  V.canvasTextEditing = id;

  let done = false, blinkOn = true, blinkTimer = 0, composing = false, stickyX: number | null = null, keepTool = false;
  let lastBlinkReset = 0;
  const restartBlink = () => {
    blinkOn = true; lastBlinkReset = performance.now();
    if (blinkTimer) window.clearInterval(blinkTimer);
    blinkTimer = window.setInterval(() => { blinkOn = !blinkOn; V.requestOverlay?.(); }, CARET_BLINK_MS);
  };

  const selection = () => ({ start: el.selectionStart ?? 0, end: el.selectionEnd ?? el.selectionStart ?? 0 });
  const publishSelection = () => {
    const { start, end } = selection();
    const previous = V.textSelection;
    if (!previous || previous.layer !== id || previous.start !== start || previous.end !== end) {
      V.textSelection = { layer: id, start, end };
      restartBlink();
      V.requestOverlay?.();
    }
  };
  const setSelection = (start: number, end: number, direction: 'forward' | 'backward' | 'none' = 'none') => {
    const length = el.value.length;
    el.setSelectionRange(Math.max(0, Math.min(length, start)), Math.max(0, Math.min(length, end)), direction);
    publishSelection();
  };

  /* Keep the hidden field over the text so IME candidate windows open near
     the glyphs, sized to the frame at the current zoom. */
  const place = () => {
    const lay = layout(); if (!lay) return;
    const frame = textFrame(lay), m = matrix();
    const origin = applyMatrix(m, { x: frame.x, y: frame.y });
    const scaleX = Math.hypot(m[0], m[1]) || 1, scaleY = Math.hypot(m[2], m[3]) || 1;
    const d = content();
    Object.assign(el.style, {
      left: `${origin.x * V.shown}px`, top: `${origin.y * V.shown}px`,
      width: `${Math.max(1, frame.w * scaleX * V.shown)}px`, height: `${Math.max(1, frame.h * scaleY * V.shown)}px`,
      fontFamily: `"${String(d?.font ?? '').replaceAll('"', '')}"`, fontSize: `${Math.max(1, lay.size * scaleY * V.shown)}px`,
      lineHeight: `${Math.max(1, lay.lineHeight * scaleY * V.shown)}px`,
      whiteSpace: lay.boxWidth > 0 ? 'pre-wrap' : 'pre',
    });
    el.wrap = lay.boxWidth > 0 ? 'soft' : 'off';
  };

  const setProperty = (path: string, value: number) => {
    api.edit.dispatch({ type: 'set_property', target: id, path, value, time: time(), mode: 'auto', preserveHandEdits: false } as any);
    api.transport.invalidate('render'); V.requestOverlay?.();
    api.services.get<{ refresh(): void }>('inspector')?.refresh();
  };
  const patchContent = (patch: Record<string, unknown>) => {
    api.edit.dispatch({ type: 'set_content', target: id, patch } as any);
    api.transport.invalidate('render'); V.requestOverlay?.();
    api.services.get<{ refresh(): void }>('inspector')?.refresh();
  };
  const write = () => {
    const value = el.value.replace(/\r/g, '');
    if (value === sourceText()) return;
    api.edit.dispatch({ type: 'set_property', target: id, path: 'c.text', value, time: time(), mode: 'auto', preserveHandEdits: false } as any);
    api.transport.invalidate('render');
  };
  /* External changes (inspector field, undo) flow back into the field. */
  const sync = () => {
    if (composing) return;
    const text = sourceText();
    if (text === el.value) return;
    const { start, end } = selection();
    el.value = text;
    setSelection(Math.min(start, text.length), Math.min(end, text.length));
  };

  const finish = () => {
    if (done) return; done = true;
    if (blinkTimer) window.clearInterval(blinkTimer);
    document.removeEventListener('selectionchange', onSelectionChange);
    document.removeEventListener('pointerdown', onDocumentPointerDown, true);
    el.remove();
    V.canvasTextEditing = null; V.finishCanvasText = null; V.textSelection = null;
    const text = sourceText();
    if (api.model.layer(id)) {
      if (!text.trim().length) {
        if (options.fresh) api.edit.cancel();
        else { api.edit.dispatch({ type: 'delete_layers', targets: [id] } as any); api.edit.commit('Delete empty text'); }
      } else {
        if (options.fresh && current().name === 'Text') api.edit.dispatch({ type: 'set_layer', target: id, patch: { name: firstLine(text) } } as any);
        api.edit.commit(options.fresh ? 'Add text' : 'Edit text');
      }
    } else api.edit.commit();
    const tools = api.services.get<{ tool: string; setTool(tool: string): void }>('tool');
    if (!keepTool && tools?.tool === 'text') tools.setTool('select');
    api.transport.invalidate();
    V.requestOverlay?.();
    api.services.get<{ refresh(): void }>('inspector')?.refresh();
    options.onFinish?.();
  };
  V.finishCanvasText = finish;

  const onSelectionChange = () => { if (document.activeElement === el) publishSelection(); };
  document.addEventListener('selectionchange', onSelectionChange);
  /* Clicks outside the viewer commit the edit before they act (a toolbar
     button still activates its tool afterwards). The inspector and any open
     popover are exempt so type settings can change while editing. */
  const onDocumentPointerDown = (e: PointerEvent) => {
    const target = e.target as Element | null;
    if (!target?.closest || target.closest('#panel-viewer, .canvas-text-input')) return;
    // Inspector value fields (text field, scrubbable numbers) open their own
    // edit transaction, so the canvas session commits first. One-shot
    // controls (font, weight, align, colour) keep the session alive.
    if (target.closest('[data-inspector-text-layer], #panel-inspector input, #panel-inspector textarea')) { finish(); return; }
    if (target.closest('#panel-inspector, [data-inspector], [role="menu"], [role="listbox"], [role="dialog"], .popover')) return;
    // A toolbar tool button is an explicit choice; its own command sets the tool.
    keepTool = !!target.closest('#toolbar button[data-tool]');
    finish();
  };
  document.addEventListener('pointerdown', onDocumentPointerDown, true);
  el.addEventListener('input', () => { write(); stickyX = null; publishSelection(); restartBlink(); V.requestOverlay?.(); });
  el.addEventListener('compositionstart', () => { composing = true; });
  el.addEventListener('compositionend', () => { composing = false; write(); publishSelection(); });
  el.addEventListener('keydown', (e: KeyboardEvent) => {
    // Other keys bubble to the field-aware global keymap, which suppresses
    // editor commands for fields and routes native shortcuts (paste) natively.
    if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); finish(); return; }
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); finish(); return; }
    if (e.key === 'Tab') { e.preventDefault(); return; }
    const mod = e.metaKey || e.ctrlKey, key = e.key.toLowerCase();
    // Select All is handled here so it works whatever the menu or keymap
    // routing does with Command+A on the way in.
    if (mod && !e.altKey && !e.shiftKey && key === 'a') { e.preventDefault(); e.stopPropagation(); setSelection(0, el.value.length); return; }
    if (mod && !e.altKey && !e.shiftKey && (key === 'b' || key === 'i')) {
      e.preventDefault(); e.stopPropagation();
      const d = content();
      if (key === 'b') setProperty('c.weight', (Number(d?.weight) || 400) >= 600 ? 400 : 700);
      else patchContent({ italic: !d?.italic });
      return;
    }
    // Command+Shift+> / < step the size; add Option for tens (Figma).
    if (mod && e.shiftKey && (e.code === 'Period' || e.code === 'Comma')) {
      e.preventDefault(); e.stopPropagation();
      const step = (e.code === 'Period' ? 1 : -1) * (e.altKey ? 10 : 1);
      setProperty('c.size', Math.max(4, Math.round((Number(content()?.size) || 16) + step)));
      return;
    }
    // Command+Option+L / T / R align left, center, right (Figma).
    if (mod && e.altKey && !e.shiftKey && (e.code === 'KeyL' || e.code === 'KeyT' || e.code === 'KeyR')) {
      e.preventDefault(); e.stopPropagation();
      patchContent({ align: e.code === 'KeyL' ? 'left' : e.code === 'KeyT' ? 'center' : 'right' });
      return;
    }
    if ((e.key === 'ArrowUp' || e.key === 'ArrowDown') && !e.altKey && !e.metaKey && !e.ctrlKey) {
      const lay = layout(); if (!lay) return;
      e.preventDefault();
      const { start, end } = selection();
      const focus = el.selectionDirection === 'backward' ? start : end;
      const anchor = el.selectionDirection === 'backward' ? end : start;
      const from = caretAt(lay, focus);
      if (stickyX == null) stickyX = from.x;
      const moved = verticalMove(lay, focus, e.key === 'ArrowUp' ? -1 : 1, stickyX);
      const next = moved ?? (e.key === 'ArrowUp' ? 0 : lay.length);
      if (e.shiftKey) setSelection(Math.min(anchor, next), Math.max(anchor, next), next < anchor ? 'backward' : 'forward');
      else if (start !== end && !e.shiftKey) setSelection(e.key === 'ArrowUp' ? start : end, e.key === 'ArrowUp' ? start : end);
      else setSelection(next, next);
      return;
    }
    if (e.key.startsWith('Arrow') || e.key === 'Home' || e.key === 'End') stickyX = null;
    window.setTimeout(publishSelection, 0);
  });
  el.addEventListener('keyup', publishSelection);

  const localPoint = (point: Point): Point | null => invertPoint(matrix(), point);
  const frameContains = (point: Point): boolean => {
    const lay = layout(); if (!lay) return false;
    const local = localPoint(point); if (!local) return false;
    const frame = textFrame(lay), slack = HIT_SLACK / Math.max(.02, V.shown);
    return local.x >= frame.x - slack && local.x <= frame.x + frame.w + slack && local.y >= frame.y - slack && local.y <= frame.y + frame.h + slack;
  };
  const indexAt = (point: Point): number | null => {
    const lay = layout(), local = localPoint(point);
    return lay && local ? indexAtPoint(lay, local.x, local.y) : null;
  };

  /* Pointer events carry no click count, so multi-click is tracked here. */
  let lastClick: { at: number; point: Point; count: number } | null = null;
  const clickCount = (event: PointerEvent, point: Point): number => {
    const now = event.timeStamp || performance.now();
    const near = lastClick && now - lastClick.at < 450
      && Math.hypot(point.x - lastClick.point.x, point.y - lastClick.point.y) * V.shown < 6;
    const count = near ? lastClick!.count + 1 : 1;
    lastClick = { at: now, point, count };
    return count;
  };

  const pointerDown = (event: PointerEvent, point: Point): boolean => {
    if (done || !frameContains(point)) return false;
    event.preventDefault();
    const index = indexAt(point); if (index == null) return true;
    const text = el.value, clicks = event.detail || clickCount(event, point);
    el.focus({ preventScroll: true });
    if (clicks >= 3) { const range = paragraphRangeAt(text, index); setSelection(range.start, range.end); return true; }
    if (clicks === 2) { const range = wordRangeAt(text, index); setSelection(range.start, range.end); return true; }
    const { start, end } = selection();
    const anchor = event.shiftKey ? (el.selectionDirection === 'backward' ? end : start) : index;
    setSelection(Math.min(anchor, index), Math.max(anchor, index), index < anchor ? 'backward' : 'forward');
    stickyX = null;
    options.drag?.(event, {
      cursor: 'text',
      move: (_dx, _dy, ev) => {
        const r = V.inner.getBoundingClientRect();
        const focus = indexAt({ x: (ev.clientX - r.left) / V.shown, y: (ev.clientY - r.top) / V.shown });
        if (focus == null) return;
        setSelection(Math.min(anchor, focus), Math.max(anchor, focus), focus < anchor ? 'backward' : 'forward');
      },
    });
    return true;
  };

  const draw = (c: CanvasRenderingContext2D, S: number, ink: string) => {
    if (done) return;
    sync(); place();
    /* A save, toast or panel refresh can drop focus to the body while the
       session is still open. Keys must keep reaching the text, so take focus
       back whenever nothing else holds it. */
    if (document.activeElement !== el && (!document.activeElement || document.activeElement === document.body) && document.hasFocus()) el.focus({ preventScroll: true });
    const lay = layout(); if (!lay) return;
    const m = matrix();
    const quad = (r: Rect) => [
      applyMatrix(m, { x: r.x, y: r.y }), applyMatrix(m, { x: r.x + r.w, y: r.y }),
      applyMatrix(m, { x: r.x + r.w, y: r.y + r.h }), applyMatrix(m, { x: r.x, y: r.y + r.h }),
    ];
    const path = (points: Point[]) => { c.beginPath(); c.moveTo(points[0]!.x, points[0]!.y); for (const p of points.slice(1)) c.lineTo(p.x, p.y); c.closePath(); };
    c.save();
    // Frame: hairline in the selection ink, with a soft fill for paragraph
    // boxes so the authored width reads while the height follows the text.
    const frame = textFrame(lay);
    path(quad(frame));
    c.strokeStyle = ink; c.lineWidth = 1 / S; c.stroke();
    if (lay.boxWidth > 0) {
      const size = 5 / S;
      for (const corner of quad(frame)) { c.fillStyle = ink; c.fillRect(corner.x - size / 2, corner.y - size / 2, size, size); }
    }
    const { start, end } = selection();
    if (end > start) {
      c.fillStyle = 'rgba(70,155,235,.38)';
      for (const rect of selectionRects(lay, start, end)) { path(quad(rect)); c.fill(); }
    } else if (blinkOn || performance.now() - lastBlinkReset < CARET_BLINK_MS) {
      const caret = caretAt(lay, start);
      const width = 1.5 / (S * (Math.hypot(m[0], m[1]) || 1));
      path(quad({ x: caret.x - width / 2, y: caret.top, w: width, h: caret.height }));
      c.fillStyle = ink; c.fill();
    }
    c.restore();
  };

  // Entry selection: caret at the clicked point, everything, or the end.
  place();
  el.focus({ preventScroll: true });
  const entryIndex = options.caretAt ? indexAt(options.caretAt) : null;
  if (options.selectAll) setSelection(0, el.value.length);
  else if (entryIndex != null) setSelection(entryIndex, entryIndex);
  else setSelection(el.value.length, el.value.length);
  restartBlink();
  api.transport.invalidate('render');
  V.requestOverlay?.();

  return { layerId: id, contains: frameContains, pointerDown, draw, finish, element: el };
}
