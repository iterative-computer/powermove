import '@powermove/tokens/tokens.css';
import '../../../css/app.css';
import './legacy/bootstrap';
import { autoEnhanceSelects } from './controls/select/enhance';

// Every native select in the app becomes a trigger with our own listbox.
autoEnhanceSelects();
