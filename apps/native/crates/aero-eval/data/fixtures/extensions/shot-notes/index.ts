import type { PowermoveAPI } from 'powermove';

const notes = new Map<string, string>();

export default function activate(api: PowermoveAPI) {
  api.panels.register({
    id: 'shot-notes.panel',
    title: 'Shot Notes',
    icon: 'note',
    build(body) {
      const project = api.project.get() as any;
      const list = document.createElement('div');
      for (const layer of project.comp.layers) {
        const row = document.createElement('label');
        row.textContent = layer.name;
        const note = document.createElement('input');
        note.value = notes.get(layer.id) ?? '';
        note.placeholder = 'Note';
        note.oninput = () => notes.set(layer.id, note.value);
        row.append(note);
        list.append(row);
      }
      body.append(list);
    }
  });
}
