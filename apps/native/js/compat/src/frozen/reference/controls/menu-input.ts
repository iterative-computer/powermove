// Powermove native compatibility adapter shared by the frozen menu controls.
const openMenus = new WeakMap<Document, () => void>();
const triggerSelector = '[aria-haspopup="menu"], [aria-haspopup="listbox"]';

export function bindMenuDismissal(
  anchor: HTMLElement,
  menu: HTMLElement,
  close: () => void,
): () => void {
  const doc = anchor.ownerDocument;
  openMenus.get(doc)?.();
  openMenus.set(doc, close);

  function onPress(event: PointerEvent): void {
    const target = event.target as Element | null;
    if (!target || menu.contains(target)) return;
    const sameTrigger = anchor.contains(target);
    const otherTrigger = event.button === 0 && target.closest?.(triggerSelector);
    if (!sameTrigger && otherTrigger && !otherTrigger.matches(':disabled, [aria-disabled="true"]')) {
      close();
      return;
    }
    event.preventDefault();
    event.stopImmediatePropagation();
    close();
    if (anchor.isConnected) anchor.focus({ preventScroll: true });
    // The browser may still produce a click after a cancelled pointerdown.
    // Retire the handler on the next physical gesture if there is no click.
    function clear(): void {
      doc.removeEventListener("click", swallowClick, true);
      doc.removeEventListener("pointerdown", clear, true);
    }
    function swallowClick(click: MouseEvent): void {
      if (click.detail === 0) return;
      clear();
      click.preventDefault();
      click.stopImmediatePropagation();
    }
    doc.addEventListener("click", swallowClick, true);
    doc.addEventListener("pointerdown", clear, true);
  }

  doc.addEventListener("pointerdown", onPress, true);
  return () => {
    if (openMenus.get(doc) === close) openMenus.delete(doc);
    doc.removeEventListener("pointerdown", onPress, true);
  };
}
