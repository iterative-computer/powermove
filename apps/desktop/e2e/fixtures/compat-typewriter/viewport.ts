import type { PowermoveAPI } from 'powermove';
import activate from './index';

// This variant opts the identity fragment shader into the viewer's cropped
// canvas contract. The ordinary entry retains the original extension's
// metadata and remains covered by the preview, sync API and cadence gates.
export default function activateViewportSafe(api: PowermoveAPI): void {
  activate(api, { viewportSafe: true });
}
