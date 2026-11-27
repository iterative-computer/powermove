export function triggerAlignment(): void;
/** Returns false when the platform/volume cannot perform this operation. */
export function cloneFile(source: string, destination: string): Promise<boolean>;
export function swapFiles(source: string, destination: string): Promise<boolean>;
export function installFile(source: string, destination: string): Promise<boolean>;
export function fullSync(fd: number): Promise<boolean>;

export function fontFamilies(): string[] | null;
/** Changes when Core Text registers or unregisters fonts; null without native notifications. */
export function fontRegistryVersion(): number | null;

/** Per pid, whether its sandbox denies a mach-lookup of `name`; null without the addon. */
export function sandboxDeniesLookup(pids: readonly number[], name: string): boolean[] | null;

export function cloudFileState(path: string, download?: boolean): Promise<'local' | 'icloud' | 'cloud' | 'missing' | 'unknown'>;

/** JSON version descriptors, or null without native support. Runs off the app thread. */
export function fileVersions(operation: 'snapshot' | 'list', path: string, contents?: string): Promise<string | null>;
