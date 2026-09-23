import '@powermove/tokens/tokens.css';
import '../../../css/app.css';
import '../../../css/settings.css';
import '../../../css/store.css';
import '../../../css/account.css';
import '../../../css/vars.css';
import './legacy/core/image-sequence.css';
import { installWebBridge, remoteLink } from './host/web-bridge';

// Served by `powermove serve`, the browser has no preload: the bridge has to be
// on window before the engines read the store during their synchronous boot.
const remote = await installWebBridge();
await import('./legacy/bootstrap');
if (remote) {
  // Other tabs on the same host edit the same document; keep this one in step.
  const [{ attachRemoteSync }, { attachRemoteFonts }] = await Promise.all([import('./host/remote-sync'), import('./host/remote-fonts')]);
  const link = remoteLink();
  const PM = (window as unknown as { PM?: Parameters<typeof attachRemoteSync>[1] & Parameters<typeof attachRemoteFonts>[1] }).PM;
  if (link && PM) {
    attachRemoteSync(link, PM);
    // Fonts travel with the project: the host keeps what any device sends.
    attachRemoteFonts(link, PM);
    // Media this browser holds that the host does not (imported before the
    // host kept copies) goes up when a project loads, so other devices see it.
    const registry = PM as unknown as { proj?: { assets?: Record<string, { storageKey?: string }> }; MediaStore?: { backfill?(keys: string[]): Promise<void> }; MediaImport?: { posterKeyFor(key: string): string }; bus: { on(event: string, fn: () => void): () => void } };
    let backfillTimer: ReturnType<typeof setTimeout> | null = null;
    const backfill = () => {
      backfillTimer = null;
      const keys = Object.values(registry.proj?.assets ?? {}).flatMap((asset) => asset.storageKey ? [asset.storageKey, registry.MediaImport?.posterKeyFor(asset.storageKey) ?? ''] : []);
      void registry.MediaStore?.backfill?.(keys.filter(Boolean));
    };
    for (const event of ['project', 'assets']) registry.bus.on(event, () => { if (!backfillTimer) backfillTimer = setTimeout(backfill, 1500); });
    backfill();
  }
}
const { autoEnhanceSelects } = await import('./controls/select/enhance');

// Every native select in the app becomes a trigger with our own listbox.
autoEnhanceSelects();
