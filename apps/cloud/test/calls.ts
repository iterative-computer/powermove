/* A stand-in for the AERO_CALLS Durable Object namespace: one AeroCall per name, each with its own storage.
   `evict` is the runtime dropping an object (a deploy, an eviction): the old instance loses its storage (every
   call on it fails from then on) and the next request or alarm gets a fresh instance over the same stored data.
   Alarms fire only when a test runs them. */
import { AeroCall, type CallState } from '../src/aero/call';
import type { Data } from '../src/db/client';

class Storage {
  fenced = false;
  /** A key whose next write kills this instance instead (the runtime dropping it mid-call). */
  dieOnPut: string | null = null;
  constructor(private readonly backing: Map<string, unknown>, private readonly alarms: Map<string, number>, private readonly name: string) {}
  private check() { if (this.fenced) throw new Error('object evicted'); }
  async get<T>(key: string) { this.check(); return structuredClone(this.backing.get(key)) as T | undefined; }
  async put(entries: Record<string, unknown>) {
    this.check();
    if (this.dieOnPut && this.dieOnPut in entries) { this.fenced = true; throw new Error('object evicted'); }
    for (const [key, value] of Object.entries(entries)) this.backing.set(key, structuredClone(value)); }
  async deleteAll() { this.check(); this.backing.clear(); }
  async setAlarm(at: number) { this.check(); this.alarms.set(this.name, at); }
  async deleteAlarm() { this.check(); this.alarms.delete(this.name); }
}

export class FakeCalls {
  readonly stored = new Map<string, Map<string, unknown>>();
  readonly alarms = new Map<string, number>();
  private readonly live = new Map<string, { call: AeroCall; storage: Storage }>();
  constructor(private readonly env: () => CloudflareBindings, private readonly data: (env: CloudflareBindings) => Data) {}

  idFromName(name: string) { return { name, toString: () => name, equals: (other: { name: string }) => other.name === name }; }
  get(id: { name: string }) { return { fetch: (input: RequestInfo, init?: RequestInit) => this.object(id.name).call.fetch(new Request(input, init)) }; }

  private object(name: string) {
    let entry = this.live.get(name);
    if (!entry) {
      if (!this.stored.has(name)) this.stored.set(name, new Map());
      const storage = new Storage(this.stored.get(name)!, this.alarms, name);
      const state: CallState = { storage, waitUntil: () => {} };
      entry = { call: new AeroCall(state, this.env(), this.data), storage };
      this.live.set(name, entry);
    }
    return entry;
  }

  /** The runtime drops the object: its memory is gone, its storage stays for the next instance. */
  evict(name: string) {
    const entry = this.live.get(name);
    if (entry) entry.storage.fenced = true;
    this.live.delete(name);
  }

  /** The next write of `key` by the object named `name` kills it (creating the object if needed). */
  dieOnPut(name: string, key: string) { this.object(name).storage.dieOnPut = key; }

  /** Fires every set alarm once (as the runtime would when they come due). Returns how many fired. */
  async runAlarms(): Promise<number> {
    const due = [...this.alarms.keys()];
    for (const name of due) { this.alarms.delete(name); await this.object(name).call.alarm(); }
    return due.length;
  }
}
