import { afterEach, expect, it, vi } from 'vitest';
import { ProjectCloudAccess, projectReadError } from './project-cloud-access';

afterEach(() => vi.useRealTimers());
const tick = async () => { for (let i = 0; i < 5; i++) await Promise.resolve(); };

it.each(['icloud', 'cloud'] as const)('starts one bounded download for a %s project without opening its data fork', async state => {
  let finish!: () => void;
  const cloud = { status: vi.fn().mockResolvedValue(state), download: vi.fn(() => new Promise<void>(resolve => { finish = resolve; })) };
  const access = new ProjectCloudAccess(cloud);
  await expect(access.prepare('/synthetic.pmv')).rejects.toThrow('Downloading this project');
  await expect(access.prepare('/synthetic.pmv')).rejects.toThrow('Downloading this project');
  expect(cloud.download).toHaveBeenCalledExactlyOnceWith('/synthetic.pmv');
  finish(); await tick();
  // Flags can lag a successful complete materialization.
  await expect(access.prepare('/synthetic.pmv')).resolves.toBeUndefined();
  expect(cloud.status).toHaveBeenCalledTimes(1);
});

it.each(['local', 'unknown', 'missing'] as const)('leaves %s files to the ordinary project validation', async state => {
  const cloud = { status: vi.fn().mockResolvedValue(state), download: vi.fn() };
  await expect(new ProjectCloudAccess(cloud).prepare('/synthetic.pmv')).resolves.toBeUndefined();
  expect(cloud.download).not.toHaveBeenCalled();
});

it('reports an offline download clearly and permits another attempt', async () => {
  const cloud = { status: vi.fn().mockResolvedValue('icloud'), download: vi.fn().mockRejectedValue(new Error('ENOTCONN')) };
  const access = new ProjectCloudAccess(cloud);
  await expect(access.prepare('/synthetic.pmv')).rejects.toThrow('Downloading');
  await tick();
  await expect(access.prepare('/synthetic.pmv')).rejects.toThrow('Could not download this project. Check your connection');
  await expect(access.prepare('/synthetic.pmv')).rejects.toThrow('Downloading');
  expect(cloud.download).toHaveBeenCalledTimes(2);
});

it('does not reuse an old download after the file is replaced or evicted again', async () => {
  const cloud = { status: vi.fn().mockResolvedValue('cloud'), download: vi.fn().mockResolvedValue(undefined), version: vi.fn().mockResolvedValue('downloaded-file') };
  const access = new ProjectCloudAccess(cloud);
  await expect(access.prepare('/synthetic.pmv')).rejects.toThrow('Downloading');
  await tick(); await tick();
  cloud.version.mockResolvedValue('replacement-placeholder');
  await expect(access.prepare('/synthetic.pmv')).rejects.toThrow('Downloading');
  expect(cloud.download).toHaveBeenCalledTimes(2);
});

it('accepts downloaded bytes with lagging cloud flags only while the file identity matches', async () => {
  const cloud = { status: vi.fn().mockResolvedValue('cloud'), download: vi.fn().mockResolvedValue(undefined), version: vi.fn().mockResolvedValue('downloaded-file') };
  const access = new ProjectCloudAccess(cloud);
  await expect(access.prepare('/synthetic.pmv')).rejects.toThrow('Downloading');
  await tick(); await tick();
  await expect(access.prepare('/synthetic.pmv')).resolves.toBeUndefined();
  expect(cloud.version).toHaveBeenCalledTimes(2);
  expect(cloud.status).toHaveBeenCalledTimes(1);
});

it('bounds metadata checks on an unresponsive network volume', async () => {
  vi.useFakeTimers();
  const cloud = { status: vi.fn(() => new Promise<never>(() => {})), download: vi.fn() };
  const checking = expect(new ProjectCloudAccess(cloud, 100).prepare('/synthetic.pmv')).rejects.toThrow('taking too long');
  await vi.advanceTimersByTimeAsync(100);
  await checking;
  expect(cloud.download).not.toHaveBeenCalled();
  expect(vi.getTimerCount()).toBe(0);
});

it('limits concurrent project downloads', async () => {
  const cloud = { status: vi.fn().mockResolvedValue('cloud'), download: vi.fn(() => new Promise<void>(() => {})) };
  const access = new ProjectCloudAccess(cloud);
  for (let i = 0; i < 4; i++) await expect(access.prepare(`/synthetic-${i}.pmv`)).rejects.toThrow('Downloading');
  await expect(access.prepare('/synthetic-5.pmv')).rejects.toThrow('Other projects');
  expect(cloud.download).toHaveBeenCalledTimes(4);
});

it('deduplicates opens whose metadata checks overlap', async () => {
  const cloud = { status: vi.fn().mockResolvedValue('cloud'), download: vi.fn(() => new Promise<void>(() => {})) };
  const access = new ProjectCloudAccess(cloud);
  const results = await Promise.allSettled([access.prepare('/synthetic.pmv'), access.prepare('/synthetic.pmv')]);
  expect(results.every(result => result.status === 'rejected')).toBe(true);
  expect(cloud.download).toHaveBeenCalledTimes(1);
});

it.each(['EDEADLK', 'Unknown system error -11', 'EAGAIN', 'ETIMEDOUT', 'ENOTCONN'])('explains a %s project read failure without calling it corruption', code => {
  expect(projectReadError({ code })).toContain('Check your connection, download the file in Finder');
});

it('preserves checksum and parsing errors and explains a temporarily missing synced file', () => {
  expect(projectReadError(new Error('Checksum does not match'))).toBe('Checksum does not match');
  expect(projectReadError({ code: 'ENOENT' })).toContain('Wait for syncing to finish');
  expect(projectReadError(null)).toBe('Could not open project.');
});
