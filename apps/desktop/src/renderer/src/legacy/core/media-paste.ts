import { isFieldTarget, selectableTextRoot } from '../../kernel/keychord';
import type { PMRegistry } from '../registry';

/** Fields (including the agent composer) own their paste events. */
export function pasteMedia(PM: PMRegistry, event: ClipboardEvent): boolean {
  if (event.defaultPrevented || event.composedPath().some(target => isFieldTarget(target)
    || selectableTextRoot(target as EventTarget))) return false;
  const transfer = event.clipboardData;
  let files = Array.from(transfer?.files ?? []);
  if (!files.length) files = Array.from(transfer?.items ?? [])
    .filter(item => item.kind === 'file').map(item => item.getAsFile()).filter((file): file is File => !!file);
  if (!files.length || typeof PM.importFiles !== 'function') return false;
  event.preventDefault();
  void PM.importFiles(files, { project: PM.proj, placement: { at: PM.time }, sequence: false });
  return true;
}
