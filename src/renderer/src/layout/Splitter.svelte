<script lang="ts">
  import type { PMRegistry } from '../legacy/registry';
  import { applyPanelSize, type DockSpec, type PanelSpec } from './model';

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
  const horizontalTarget = () => {
    const useBefore = !isFlexPanel(beforeSpec);
    return {
      spec: useBefore ? beforeSpec! : afterSpec!,
      other: useBefore ? afterSpec! : beforeSpec!,
      sign: useBefore ? 1 : -1
    };
  };
  const panelElement = (spec: PanelSpec): HTMLElement | null => PM.panelInst[spec.id]?.el ?? null;
  const readValue = (): number => {
    if (mode === 'vertical') {
      const { target } = verticalTarget();
      return Math.round(document.getElementById(`dock-${target.id}`)?.getBoundingClientRect().width || target.size || (target.id === 'right' ? 300 : 250));
    }
    const { spec } = horizontalTarget();
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
  const horizontalGeometry = () => {
    const { spec, other, sign } = horizontalTarget();
    const element = panelElement(spec);
    const otherElement = panelElement(other);
    if (!element || !otherElement) return null;
    const start = element.getBoundingClientRect().height;
    const otherStart = otherElement.getBoundingClientRect().height;
    const gap = Number.parseFloat(window.getComputedStyle(splitter).height) || 8;
    const min = spec.min || PM.PANELS[spec.id]?.min || 88;
    const otherMin = other.min || PM.PANELS[other.id]?.min || 88;
    return { spec, other, sign, element, otherElement, start, pairHeight: start + otherStart, gap, min, otherMin };
  };
  const setHorizontal = (geometry: NonNullable<ReturnType<typeof horizontalGeometry>>, delta: number): void => {
    const height = PM.Layout.clampPanelHeight(
      geometry.start,
      delta,
      geometry.sign,
      geometry.pairHeight,
      geometry.min,
      geometry.otherMin,
      geometry.gap
    );
    geometry.element.style.flex = `0 0 ${height}px`;
    geometry.spec.size = Math.round(height);
    delete geometry.spec.flex;
    geometry.other.flex = true;
    delete geometry.other.size;
    geometry.otherElement.style.flex = '1 1 auto';
    value = Math.round(height);
    splitter?.setAttribute('aria-valuenow', String(value));
    const min = Math.max(72, Number(geometry.min) || 88);
    minimum = min;
    maximum = Math.max(min, geometry.pairHeight - Math.max(72, Number(geometry.otherMin) || 88) - geometry.gap);
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
    const min = Math.max(72, Number(geometry.min) || 88);
    minimum = min;
    maximum = Math.max(min, geometry.pairHeight - Math.max(72, Number(geometry.otherMin) || 88) - geometry.gap);
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
