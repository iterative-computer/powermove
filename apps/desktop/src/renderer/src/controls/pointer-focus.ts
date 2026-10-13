/** Pointer clicks must not leave an action selected for subsequent Space presses.
 * Run before action handlers so a dialog or text field they focus keeps its focus.
 * Keyboard activation has detail=0 and retains normal keyboard navigation.
 */
export function installPointerFocus(doc: Document = document): () => void {
  const release = (event: MouseEvent): void => {
    if (event.detail === 0) return;
    const control = event.composedPath().find((node): node is HTMLElement =>
      node instanceof HTMLElement && node.matches('button,[role="button"],[role="tab"],[role="radio"],[role="menuitem"]'));
    if (!control) return;
    let active = doc.activeElement;
    while (active?.shadowRoot?.activeElement) active = active.shadowRoot.activeElement;
    if (active === control) control.blur();
  };
  doc.addEventListener('click', release, true);
  return () => doc.removeEventListener('click', release, true);
}
