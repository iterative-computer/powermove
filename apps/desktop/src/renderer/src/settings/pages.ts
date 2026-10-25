/** The Settings sidebar's pages (SettingsScreen.svelte), in a plain module
 *  so the installer's type check needs no Svelte component types. */
export type SettingsPage = 'general' | 'accounts' | 'transcription' | 'extensions' | 'project';
