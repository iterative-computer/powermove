/* Settings › General › Media › Generated media location (appendix D): a
   select of Powermove cache, Next to the project, the chosen folder when
   there is one, and Choose Folder…. */
import type { GeneratedMediaLocation } from './proxies-types';

export type LocationChoice = 'cache' | 'project' | 'folder' | 'choose';

export function locationChoice(location: GeneratedMediaLocation): LocationChoice {
  return location.kind;
}

/** The folder's own name, as Finder shows it. */
export function folderName(path: string): string {
  const parts = path.split('/').filter(Boolean);
  return parts.at(-1) ?? '/';
}

export function locationOptions(location: GeneratedMediaLocation | null): Array<{ v: LocationChoice; label: string }> {
  return [
    { v: 'cache', label: 'Powermove cache' },
    { v: 'project', label: 'Next to the project' },
    ...(location?.kind === 'folder' ? [{ v: 'folder' as const, label: folderName(location.path) }] : []),
    { v: 'choose', label: 'Choose Folder…' }
  ];
}
