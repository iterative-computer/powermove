import '../../../css/tokens.css';
import '../../../css/app.css';
import './legacy/bootstrap';

/* Input-modality tracking: focus rings only appear for keyboard navigation.
   Pointer clicks never leave an accent outline on buttons (native feel). */
document.documentElement.dataset.modality = 'pointer';
window.addEventListener('keydown', (e) => {
  if (e.key === 'Tab' || e.key.startsWith('Arrow')) document.documentElement.dataset.modality = 'keyboard';
}, true);
window.addEventListener('pointerdown', () => {
  document.documentElement.dataset.modality = 'pointer';
}, true);
