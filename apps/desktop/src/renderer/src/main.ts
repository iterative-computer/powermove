import '@powermove/tokens/tokens.css';
import '../../../css/app.css';
import '../../../css/settings.css';
import './legacy/core/image-sequence.css';
import './legacy/bootstrap';
import { autoEnhanceSelects } from './controls/select/enhance';

// Every native select in the app becomes a trigger with our own listbox.
autoEnhanceSelects();
