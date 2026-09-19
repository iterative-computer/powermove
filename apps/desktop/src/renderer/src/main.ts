import '@powermove/tokens/tokens.css';
import '../../../css/app.css';
import '../../../css/settings.css';
import './legacy/core/image-sequence.css';
import { installWebBridge } from './host/web-bridge';

// Served by `powermove serve`, the browser has no preload: the bridge has to be
// on window before the engines read the store during their synchronous boot.
await installWebBridge();
await import('./legacy/bootstrap');
const { autoEnhanceSelects } = await import('./controls/select/enhance');

// Every native select in the app becomes a trigger with our own listbox.
autoEnhanceSelects();
