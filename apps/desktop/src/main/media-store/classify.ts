/*
 * Which imports are copied even when a project links files (plan section
 * 21, "Always copied"):
 *
 *  - files inside an agent workspace (the workspace is deleted with its
 *    project, `legacy/core/projects.ts`), and anything else under the app's
 *    own data folder, which the app manages and may remove;
 *  - files on removable camera media: a card or other removable medium
 *    (macOS reports it as removable media), or any mounted volume laid out
 *    like a camera card (DCIM, AVCHD, XDCAM, P2, Canon and Sony layouts).
 *
 * Items without a file (clipboard images, downloads, generated stills),
 * Blender sources, 3D models, and agent imports without `storage: "link"`
 * are decided by the renderer, which knows what it is importing.
 */
import { execFile } from 'node:child_process';
import { realpath, stat } from 'node:fs/promises';
import path from 'node:path';

import type { ClassifyResult } from '../../shared/media-store';

/** Folders at a volume's root that only camera cards have. */
export const CAMERA_CARD_MARKERS: readonly string[] = [
  'DCIM',
  'AVCHD',
  'BPAV',
  'XDROOT',
  'MP_ROOT',
  'PRIVATE/AVCHD',
  'PRIVATE/M4ROOT',
  'PRIVATE/XDROOT',
  'PRIVATE/PANA_GRP',
  'CONTENTS/CLIP',
  'CONTENTS/CLIPS001'
];

export type DiskInfo = { removableMedia: boolean; internal: boolean };

export type ClassifyOptions = {
  /** The app's data folder (`app.getPath('userData')`). */
  userData: string;
  /** `diskutil info` for a mount point; injectable for tests. */
  diskInfo?: (mountPoint: string) => Promise<DiskInfo | null>;
  exists?: (file: string) => Promise<boolean>;
  /** The mount point holding `file`; injectable for tests. */
  mountPoint?: (file: string) => Promise<string>;
};

const defaultExists = async (file: string) => { try { await stat(file); return true; } catch { return false; } };

/** The deepest ancestor on the same device as `file`. */
export async function mountPointOf(file: string): Promise<string> {
  let current = path.dirname(file);
  let device = (await stat(file)).dev;
  for (;;) {
    const parent = path.dirname(current);
    if (parent === current) return current;
    let info;
    try { info = await stat(parent); } catch { return current; }
    if (info.dev !== device) {
      // `current` may itself be on another device than `file` (a nested mount).
      return current;
    }
    device = info.dev;
    current = parent;
  }
}

/** `diskutil info -plist` reduced to what classification needs. */
export function diskutilInfo(mountPoint: string): Promise<DiskInfo | null> {
  if (process.platform !== 'darwin') return Promise.resolve(null);
  return new Promise((resolve) => {
    execFile('/usr/sbin/diskutil', ['info', '-plist', mountPoint], { timeout: 5000, maxBuffer: 1024 * 1024 }, (error, stdout) => {
      if (error) { resolve(null); return; }
      const flag = (name: string) => new RegExp(`<key>${name}</key>\\s*<true/>`).test(stdout);
      resolve({ removableMedia: flag('RemovableMedia'), internal: flag('Internal') });
    });
  });
}

const within = (child: string, parent: string) => {
  const relative = path.relative(parent, child);
  return relative === '' || (!!relative && !relative.startsWith('..') && !path.isAbsolute(relative));
};

export class ImportClassifier {
  private readonly volumes = new Map<string, Promise<boolean>>();
  private readonly diskInfo: NonNullable<ClassifyOptions['diskInfo']>;
  private readonly exists: NonNullable<ClassifyOptions['exists']>;
  private readonly mountPoint: NonNullable<ClassifyOptions['mountPoint']>;

  constructor(private readonly options: ClassifyOptions) {
    this.diskInfo = options.diskInfo ?? diskutilInfo;
    this.exists = options.exists ?? defaultExists;
    this.mountPoint = options.mountPoint ?? mountPointOf;
  }

  async classify(file: string): Promise<ClassifyResult> {
    if (typeof file !== 'string' || !path.isAbsolute(file)) return { storage: 'copied', reason: 'notAFile' };
    let resolved: string;
    try {
      resolved = await realpath(file);
      if (!(await stat(resolved)).isFile()) return { storage: 'copied', reason: 'notAFile' };
    } catch { return { storage: 'copied', reason: 'notAFile' }; }
    let userData = this.options.userData;
    try { userData = await realpath(userData); } catch { /* not created yet */ }
    if (within(resolved, userData) || within(file, this.options.userData)) return { storage: 'copied', reason: 'agentWorkspace' };
    if (await this.onCameraMedia(resolved)) return { storage: 'copied', reason: 'cameraMedia' };
    return { storage: 'linked' };
  }

  private async onCameraMedia(file: string): Promise<boolean> {
    let mount: string;
    try { mount = await this.mountPoint(file); } catch { return false; }
    // The startup volume is never camera media.
    if (mount === '/' || mount === '/System/Volumes/Data') return false;
    let known = this.volumes.get(mount);
    if (!known) {
      known = (async () => {
        const info = await this.diskInfo(mount);
        if (info?.removableMedia) return true;
        for (const marker of CAMERA_CARD_MARKERS) if (await this.exists(path.join(mount, marker))) return true;
        return false;
      })();
      this.volumes.set(mount, known);
      // A card can be swapped under the same mount point.
      setTimeout(() => { if (this.volumes.get(mount) === known) this.volumes.delete(mount); }, 60_000).unref?.();
    }
    return known;
  }
}
