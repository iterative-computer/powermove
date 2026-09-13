<script lang="ts">
  import { onMount } from 'svelte';
  import type { PMRegistry } from '../legacy/registry';
  import { clampPanelHeight, resolvePairResize, transferPanelHeights } from './geometry';
  import { applyPanelSize, panelMinHeight, setPanelCollapsed, type DockSpec, type PanelSpec } from './model';

  let {
    PM,
    mode,
    beforeDock,
    afterDock,
    beforeSpec,
    afterSpec,
    tick,
    oncommit
  }: {
    PM: PMRegistry;
    mode: 'vertical' | 'horizontal';
    beforeDock?: DockSpec;
    afterDock?: DockSpec;
    beforeSpec?: PanelSpec;
    afterSpec?: PanelSpec;
    tick: number;
    oncommit: () => void;
  } = $props();

  let splitter: HTMLElement;
  let value = $state(0);
  let minimum = $state<number | undefined>(undefined);
  let maximum = $state<number | undefined>(undefined);

  const isFlexDock = (dock?: DockSpec): boolean => !dock || dock.id === 'center' || !!dock.flex;
  const isFlexPanel = (spec?: PanelSpec): boolean => {
    if (!spec) return true;
    const def = PM.PANELS[spec.id] || {};
    return !!(spec.flex || (!spec.size && !def.size));
  };
  const verticalTarget = () => {
    const useBefore = !isFlexDock(beforeDock);
    return { target: useBefore ? beforeDock! : afterDock!, sign: useBefore ? 1 : -1 };
  };
  // Column dividers are siblings of the docks, so native wheel scrolling
  // cannot reach the adjacent side column from their hit area.
  onMount(() => {
    const onWheel = (event: WheelEvent): void => {
      if (mode !== 'vertical' || event.ctrlKey || event.metaKey || event.shiftKey || !event.deltaY) return;
      const target = [beforeDock, afterDock].find((dock) => dock?.id === 'left' || dock?.id === 'right');
      const dock = target && document.getElementById(`dock-${target.id}`);
      if (!dock || dock.scrollHeight <= dock.clientHeight) return;
      const unit = event.deltaMode === WheelEvent.DOM_DELTA_PAGE ? dock.clientHeight
        : event.deltaMode === WheelEvent.DOM_DELTA_LINE ? 16 : 1;
      event.preventDefault();
      dock.scrollTop += event.deltaY * unit;
    };
    splitter.addEventListener('wheel', onWheel, { passive: false });
    return () => splitter.removeEventListener('wheel', onWheel);
  });

  const horizontalPair = () => {
    const before = beforeSpec!;
    const after = afterSpec!;
    // In a multi-panel dock, changing one neighbour back to flex redistributes
    // space to unrelated siblings. Pin both neighbours and conserve their
    // combined measured height so only this divider's pair can move.
    const dock = splitter?.closest('.dock');
    if ((dock?.querySelectorAll(':scope > .panel-slot').length ?? 0) > 2) {
      return { resize: { mode: 'transfer' as const }, spec: before, other: after, sign: 1 as const };
    }
    const resize = resolvePairResize(isFlexPanel(before), isFlexPanel(after));
    if (resize.mode === 'transfer') return { resize, spec: before, other: after, sign: 1 as const };
    const spec = resize.target === 'before' ? before : after;
    const other = resize.target === 'before' ? after : before;
    return { resize, spec, other, sign: resize.sign };
  };
  const panelElement = (spec: PanelSpec): HTMLElement | null => PM.panelInst[spec.id]?.el ?? null;
  const readValue = (): number => {
    if (mode === 'vertical') {
      const { target } = verticalTarget();
      return Math.round(document.getElementById(`dock-${target.id}`)?.getBoundingClientRect().width || target.size || (target.id === 'right' ? 300 : 250));
    }
    const { spec } = horizontalPair();
    return Math.round(panelElement(spec)?.getBoundingClientRect().height || spec.size || PM.PANELS[spec.id]?.size || 180);
  };

  $effect(() => {
    void tick;
    value = readValue();
    minimum = mode === 'vertical' ? 200 : undefined;
    maximum = mode === 'vertical' ? 760 : undefined;
  });

  let frame = 0;
  const emitAtFrameCadence = (): void => {
    if (frame) return;
    frame = window.requestAnimationFrame(() => {
      frame = 0;
      // install-legacy forwards this exact legacy event to frameBus.
      PM.bus.emit('layout:applied');
    });
  };
  const finishCommit = (): void => {
    PM.WS.save();
    oncommit();
    PM.bus.emit('layout');
    PM.bus.emit('layout:applied');
  };
  const setVertical = (next: number): void => {
    const { target } = verticalTarget();
    const clamped = PM.clamp(next, 200, 760);
    const element = document.getElementById(`dock-${target.id}`);
    if (!element) return;
    element.style.flex = `0 0 ${clamped}px`;
    target.size = Math.round(clamped);
    target.flex = false;
    value = Math.round(clamped);
    splitter?.setAttribute('aria-valuenow', String(value));
    minimum = 200;
    maximum = 760;
  };
  const panelMin = (spec: PanelSpec): number => panelMinHeight(spec, PM.PANELS[spec.id] || {});
  const horizontalGeometry = () => {
    const { resize, spec, other, sign } = horizontalPair();
    const element = panelElement(spec);
    const otherElement = panelElement(other);
    if (!element || !otherElement) return null;
    const start = element.getBoundingClientRect().height;
    const otherStart = otherElement.getBoundingClientRect().height;
    const gap = resize.mode === 'transfer' ? 0 : Number.parseFloat(window.getComputedStyle(splitter).height) || 8;
    return {
      resize, spec, other, sign, element, otherElement, start, otherStart,
      pairHeight: start + otherStart, gap, min: panelMin(spec), otherMin: panelMin(other)
    };
  };
  const pin = (element: HTMLElement, spec: PanelSpec, height: number): void => {
    spec.size = Math.round(height);
    delete spec.flex;
    applyPanelSize(element, spec, PM.PANELS[spec.id] || {});
  };
  const setHorizontal = (geometry: NonNullable<ReturnType<typeof horizontalGeometry>>, delta: number): void => {
    let height: number;
    if (geometry.resize.mode === 'transfer') {
      const next = transferPanelHeights(geometry.start, geometry.otherStart, delta, geometry.min, geometry.otherMin);
      pin(geometry.element, geometry.spec, next.before);
      pin(geometry.otherElement, geometry.other, next.after);
      height = next.before;
    } else {
      height = clampPanelHeight(geometry.start, delta, geometry.sign, geometry.pairHeight, geometry.min, geometry.otherMin, geometry.gap);
      pin(geometry.element, geometry.spec, height);
      geometry.other.flex = true;
      delete geometry.other.size;
      geometry.otherElement.style.flex = '1 1 auto';
    }
    value = Math.round(height);
    splitter?.setAttribute('aria-valuenow', String(value));
    minimum = geometry.min;
    maximum = Math.max(geometry.min, geometry.pairHeight - geometry.otherMin - geometry.gap);
  };

  let hoverCurrent: number | null = null;
  let hoverTarget: number | null = null;
  let hoverFrame = 0;
  const paintHover = (): void => {
    if (hoverCurrent == null || hoverTarget == null) {
      hoverFrame = 0;
      return;
    }
    hoverCurrent += (hoverTarget - hoverCurrent) * 0.36;
    if (Math.abs(hoverTarget - hoverCurrent) < 0.1) hoverCurrent = hoverTarget;
    const property = mode === 'horizontal' ? '--splitter-hover-x' : '--splitter-hover-y';
    splitter.style.setProperty(property, `${hoverCurrent.toFixed(2)}px`);
    hoverFrame = hoverCurrent === hoverTarget ? 0 : window.requestAnimationFrame(paintHover);
  };
  function trackPointer(event: PointerEvent): void {
    const rect = splitter.getBoundingClientRect();
    const horizontal = mode === 'horizontal';
    const length = horizontal ? rect.width : rect.height;
    const pointer = horizontal ? event.clientX - rect.left : event.clientY - rect.top;
    const nextTarget = PM.clamp(pointer, 0, length);
    hoverTarget = nextTarget;
    if (hoverCurrent == null) {
      hoverCurrent = nextTarget;
      splitter.style.setProperty(horizontal ? '--splitter-hover-x' : '--splitter-hover-y', `${nextTarget.toFixed(2)}px`);
    } else if (!hoverFrame) {
      hoverFrame = window.requestAnimationFrame(paintHover);
    }
  }

  function exposeBounds(): void {
    if (mode === 'vertical') {
      minimum = 200;
      maximum = 760;
      return;
    }
    const geometry = horizontalGeometry();
    if (!geometry) return;
    minimum = geometry.min;
    maximum = Math.max(geometry.min, geometry.pairHeight - geometry.otherMin - geometry.gap);
  }

  /* Dragging a splitter next to a collapsed panel means "give me that panel
     back": expand it first so the drag has real geometry to work with. */
  function expandCollapsedNeighbours(): void {
    for (const spec of [beforeSpec, afterSpec]) {
      if (spec?.collapsed) setPanelCollapsed(PM, spec.id, false, false);
    }
  }

  function onPointerDown(event: PointerEvent): void {
    if (event.button !== 0) return;
    event.stopPropagation();
    exposeBounds();
    trackPointer(event);
    splitter.classList.add('drag');
    if (mode === 'vertical') {
      const { target, sign } = verticalTarget();
      const element = document.getElementById(`dock-${target.id}`);
      if (!element) {
        splitter.classList.remove('drag');
        return;
      }
      const start = element.getBoundingClientRect().width;
      const saved = { size: target.size, flex: target.flex };
      PM.drag(event, {
        cursor: 'col-resize',
        move: (dx: number, _dy: number, next: PointerEvent) => {
          trackPointer(next);
          setVertical(start + sign * dx);
          emitAtFrameCadence();
        },
        up: () => {
          splitter.classList.remove('drag');
          finishCommit();
        },
        cancel: () => {
          splitter.classList.remove('drag');
          if (saved.size == null) delete target.size; else target.size = saved.size;
          if (saved.flex == null) delete target.flex; else target.flex = saved.flex;
          element.style.flex = isFlexDock(target) ? '1 1 auto' : `0 0 ${target.size || (target.id === 'right' ? 300 : 250)}px`;
          PM.bus.emit('layout:applied');
        }
      });
      return;
    }

    expandCollapsedNeighbours();
    const geometry = horizontalGeometry();
    if (!geometry) {
      splitter.classList.remove('drag');
      return;
    }
    const saved = { size: geometry.spec.size, flex: geometry.spec.flex };
    const otherSaved = { size: geometry.other.size, flex: geometry.other.flex };
    let changed = false;
    PM.drag(event, {
      cursor: 'row-resize',
      move: (_dx: number, dy: number, next: PointerEvent) => {
        trackPointer(next);
        if (!changed && Math.abs(dy) < 2) return;
        changed = true;
        setHorizontal(geometry, dy);
        emitAtFrameCadence();
      },
      up: () => {
        splitter.classList.remove('drag');
        if (changed) finishCommit();
      },
      cancel: () => {
        splitter.classList.remove('drag');
        if (!changed) return;
        if (saved.size == null) delete geometry.spec.size; else geometry.spec.size = saved.size;
        if (saved.flex == null) delete geometry.spec.flex; else geometry.spec.flex = saved.flex;
        if (otherSaved.size == null) delete geometry.other.size; else geometry.other.size = otherSaved.size;
        if (otherSaved.flex == null) delete geometry.other.flex; else geometry.other.flex = otherSaved.flex;
        applyPanelSize(geometry.element, geometry.spec, PM.PANELS[geometry.spec.id] || {});
        applyPanelSize(geometry.otherElement, geometry.other, PM.PANELS[geometry.other.id] || {});
        PM.bus.emit('layout:applied');
      }
    });
  }

  function resetVertical(): void {
    if (mode !== 'vertical') return;
    const { target } = verticalTarget();
    const next = target.id === 'right' ? 300 : 250;
    const element = document.getElementById(`dock-${target.id}`);
    if (!element) return;
    element.style.flex = `0 0 ${next}px`;
    target.size = next;
    value = next;
    splitter?.setAttribute('aria-valuenow', String(value));
    PM.WS.save();
    PM.bus.emit('layout:applied');
  }

  function onKeyDown(event: KeyboardEvent): void {
    if (event.key === 'Enter' && mode === 'vertical') {
      event.preventDefault();
      resetVertical();
      return;
    }
    const delta = mode === 'vertical'
      ? event.key === 'ArrowRight' ? 10 : event.key === 'ArrowLeft' ? -10 : null
      : event.key === 'ArrowDown' ? 10 : event.key === 'ArrowUp' ? -10 : null;
    if (delta == null) return;
    event.preventDefault();
    if (mode === 'vertical') {
      const { sign } = verticalTarget();
      setVertical(value + sign * delta);
    } else {
      expandCollapsedNeighbours();
      const geometry = horizontalGeometry();
      if (!geometry) return;
      setHorizontal(geometry, delta);
    }
    finishCommit();
  }
</script>

<!-- The ARIA separator pattern is intentionally focusable and keyboard-driven. -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  bind:this={splitter}
  class:splitter={true}
  class:h={mode === 'horizontal'}
  role="separator"
  aria-orientation={mode}
  aria-label={mode === 'vertical' ? 'Resize dock' : 'Resize panels'}
  aria-valuenow={value}
  aria-valuemin={minimum}
  aria-valuemax={maximum}
  tabindex="0"
  onpointerenter={trackPointer}
  onpointermove={trackPointer}
  onpointerdown={onPointerDown}
  onfocus={exposeBounds}
  ondblclick={resetVertical}
  onkeydown={onKeyDown}
></div>
