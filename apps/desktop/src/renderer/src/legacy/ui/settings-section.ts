export interface SettingsSection {
  element: HTMLElement;
  /** Muted line beside the title; rewrite it to report counts or errors. */
  summary: HTMLElement;
  /** Bordered card the section's rows live in. */
  body: HTMLElement;
}

/** Heading plus card body shared by every imperative section of Settings. */
export function createSettingsSection(title: string, summary = ''): SettingsSection {
  const element = document.createElement('section');
  element.className = 'settings-section';
  const heading = document.createElement('div');
  heading.className = 'settings-section-heading';
  const label = document.createElement('b');
  label.textContent = title;
  const note = document.createElement('span');
  note.textContent = summary;
  heading.append(label, note);
  const body = document.createElement('div');
  body.className = 'settings-section-body sg-group';
  element.append(heading, body);
  return { element, summary: note, body };
}
