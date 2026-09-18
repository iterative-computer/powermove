import './import-progress.css';

export type ImportProgress = { label: string; completed?: number; total?: number };

/** A persistent, non-modal status surface, owned by one import operation. */
export function createImportProgress(title: string) {
  const card = document.createElement('section');
  card.className = 'import-progress';
  card.setAttribute('aria-label', title);
  const heading = document.createElement('strong');
  heading.textContent = title;
  const detail = document.createElement('div');
  detail.className = 'import-progress-detail';
  const label = document.createElement('span');
  label.setAttribute('role', 'status');
  const amount = document.createElement('span');
  amount.className = 'import-progress-amount';
  amount.setAttribute('aria-hidden', 'true');
  const bar = document.createElement('progress');
  bar.max = 1;
  detail.append(label, amount);
  card.append(heading, detail, bar);
  document.body.append(card);
  return {
    update({ label: text, completed, total }: ImportProgress) {
      label.textContent = text;
      label.title = text;
      bar.setAttribute('aria-label', text);
      if (completed != null && total != null && total > 0) {
        bar.value = Math.min(1, Math.max(0, completed / total));
        amount.textContent = `${Math.round(bar.value * 100)}%`;
      } else {
        bar.removeAttribute('value');
        amount.textContent = '';
      }
    },
    close() { card.remove(); },
  };
}
