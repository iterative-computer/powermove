/**
 * Transitional registry for the legacy module port. Per-module concrete types
 * replace `any` as each subsystem's public contract is migrated.
 */
export interface PMRegistry extends Record<string, any> {}
