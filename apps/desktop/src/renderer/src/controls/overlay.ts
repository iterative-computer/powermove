/**
 * Move a control overlay out of its panel's clipped scrolling surface.
 *
 * Picker geometry is viewport-relative, so the document body is also the
 * correct containing block. Svelte still owns and cleans up the moved node.
 */
export function mountOverlayOnBody(node: HTMLElement): { destroy(): void } {
  document.body.appendChild(node);
  return {
    destroy() {
      node.remove();
    }
  };
}

/** Keep pickers on one side of their trigger; scroll the body when neither side fits. */
export function anchorPicker(node: HTMLElement, trigger: HTMLElement | undefined): { destroy(): void } {
  const margin = 12;
  const gap = 6;
  function position(): void {
    if (!trigger) return;
    const anchor = trigger.getBoundingClientRect();
    node.style.maxHeight = '';
    node.style.maxWidth = `${Math.max(0, window.innerWidth - margin * 2)}px`;
    // Layout size, not the transformed box: the picker may be mid-scale as it opens.
    const bounds = { width: node.offsetWidth, height: node.offsetHeight };
    const below = Math.max(0, window.innerHeight - margin - anchor.bottom - gap);
    const above = Math.max(0, anchor.top - margin - gap);
    const useBelow = bounds.height <= below || (bounds.height > above && below >= above);
    const available = useBelow ? below : above;
    // Enter/exit motion grows from the trigger's edge, like every dropdown menu.
    node.dataset.side = useBelow ? 'bottom' : 'top';
    node.style.maxHeight = `${available}px`;
    const height = Math.min(bounds.height, available);
    node.style.left = `${Math.max(margin, Math.min(anchor.right - bounds.width, window.innerWidth - bounds.width - margin))}px`;
    node.style.top = `${useBelow ? anchor.bottom + gap : anchor.top - gap - height}px`;
  }
  position();
  const observer = new ResizeObserver(position);
  observer.observe(node);
  if (trigger) observer.observe(trigger);
  window.addEventListener('resize', position);
  window.addEventListener('scroll', position, true);
  return {
    destroy() {
      observer.disconnect();
      window.removeEventListener('resize', position);
      window.removeEventListener('scroll', position, true);
    }
  };
}
