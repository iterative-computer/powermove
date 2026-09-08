import '../../../../css/tokens.css';
import './welcome.css';

const button = document.querySelector<HTMLButtonElement>('#begin');
const status = document.querySelector<HTMLElement>('#begin-status');

button?.addEventListener('click', async () => {
  button.disabled = true;
  if (status) status.textContent = 'Opening your workspace…';
  try {
    await window.onboarding.begin();
  } catch {
    button.disabled = false;
    if (status) status.textContent = 'Could not open Powermove. Try again.';
  }
});

button?.focus();
