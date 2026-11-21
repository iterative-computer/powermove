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
    // A cancelled pointerdown can still produce a click or context menu.
    // Retire the handlers when an unrelated gesture starts.
    const contextPress = event.button === 2 || (event.button === 0 && event.ctrlKey);
    function clear(): void {
      doc.removeEventListener("click", swallowClick, true);
      doc.removeEventListener("auxclick", swallowClick, true);
      doc.removeEventListener("contextmenu", swallowContextMenu, true);
      doc.removeEventListener("pointerdown", clear, true);
      doc.removeEventListener("keydown", clear, true);
    }
    function swallowContextMenu(context: MouseEvent): void {
      context.preventDefault();
      context.stopImmediatePropagation();
    }
    function swallowClick(click: MouseEvent): void {
      if (click.button !== event.button || (click.type === "click" && click.detail === 0)) return;
      clear();
      click.preventDefault();
      click.stopImmediatePropagation();
    }
    doc.addEventListener("click", swallowClick, true);
    doc.addEventListener("auxclick", swallowClick, true);
    if (contextPress) doc.addEventListener("contextmenu", swallowContextMenu, true);
    doc.addEventListener("pointerdown", clear, true);
    doc.addEventListener("keydown", clear, true);
  }

  doc.addEventListener("pointerdown", onPress, true);
  return () => {
    if (openMenus.get(doc) === close) openMenus.delete(doc);
    doc.removeEventListener("pointerdown", onPress, true);
  };
}
