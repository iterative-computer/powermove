export function triggerAlignment(): void;

export function fontFamilies(): string[] | null;

export function cloudFileState(path: string, download?: boolean): Promise<'local' | 'icloud' | 'cloud' | 'missing' | 'unknown'>;
