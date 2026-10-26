export const STORAGE_IPC = {
  status: 'storage:status',
  cleanup: 'storage:cleanup',
  configure: 'storage:configure',
  reveal: 'storage:reveal',
  upgrade: 'storage:upgrade',
  upgradeProgress: 'storage:upgrade-progress'
} as const;

export const BACKUP_BUDGETS = [1, 5, 10, 20] as const;
export type StorageSettings = { automatic: boolean; backupBudgetGiB: number };
export const DEFAULT_STORAGE_SETTINGS: StorageSettings = { automatic: true, backupBudgetGiB: 5 };
export type StorageCategory = { id: string; label: string; bytes: number; reclaimable: number };
export type StorageStatus = {
  settings: StorageSettings;
  totalBytes: number;
  reclaimableBytes: number;
  categories: StorageCategory[];
  backupBudgetExceeded: boolean;
};
export type StorageCleanupResult = { freedBytes: number; warnings: string[]; status: StorageStatus };
export type StorageUpgradeStage = 'recovery' | 'browser' | 'sources' | 'cleanup';
export type StorageUpgradeProgress = { stage: StorageUpgradeStage; progress: number };
export type StorageUpgradeRequest = { action: 'start' | 'browserComplete' | 'notified'; token?: string };
export type StorageUpgradeResult = {
  state: 'busy' | 'browser' | 'complete' | 'paused';
  token?: string;
  notify?: boolean;
  freedBytes?: number;
};
export interface StorageBridge {
  status(): Promise<StorageStatus>;
  cleanup(): Promise<StorageCleanupResult>;
  configure(settings: StorageSettings): Promise<StorageStatus>;
  reveal(): Promise<void>;
  upgrade?(request: StorageUpgradeRequest): Promise<StorageUpgradeResult>;
  onUpgradeProgress?(listener: (progress: StorageUpgradeProgress) => void): () => void;
}
