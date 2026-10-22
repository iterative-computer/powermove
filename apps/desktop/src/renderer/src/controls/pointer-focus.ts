/** Pointer clicks must not leave an action selected for subsequent Space presses.
 * Run before action handlers so a dialog or text field they focus keeps its focus.
 * Keyboard activation has detail=0 and retains normal keyboard navigation.
 * A control inside a focusable dialog hands focus to that dialog rather than to
 * <body>: the dialog's Escape and Tab handling listen on it, and a focused
 * dialog keeps keys away from the editor shortcuts behind it.
 */
export function installPointerFocus(doc: Document = document): () => void {
  const focused = (): Element | null => {
    let active = doc.activeElement;
    while (active?.shadowRoot?.activeElement) active = active.shadowRoot.activeElement;
    return active;
  };
  const release = (event: MouseEvent): void => {
    if (event.detail === 0) return;
    const control = event.composedPath().find((node): node is HTMLElement =>
      node instanceof HTMLElement && node.matches('button,[role="button"],[role="tab"],[role="radio"],[role="menuitem"]'));
    if (!control || focused() !== control) return;
    const dialog = control.parentElement?.closest<HTMLElement>('[role="dialog"],[role="alertdialog"]');
    if (dialog?.hasAttribute('tabindex')) dialog.focus({ preventScroll: true });
    // A dialog that could not take focus (inert, hidden) must still not leave the control focused.
    if (focused() === control) control.blur();
  };
  doc.addEventListener('click', release, true);
  return () => doc.removeEventListener('click', release, true);
}
