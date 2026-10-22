/* Local incremental integrity digest, byte-for-byte with pm-model/consistency.rs.
 * The canonical wire SHA-256 is unchanged. Only JSON shadows enter this cache;
 * live mutable aliases are compared against the shadow before using it. */
import { canonicalHash } from '../../../shared/canonical-json';
import type { PatchOp } from '../../../shared/patch-ids';

export class ConsistencyHash {
  private cache = new WeakMap<object, Map<number, string>>();
  hashed = 0;
  reused = 0;

  /** applyOps mutates containers in place. Invalidate all existing ancestors
   * before any operation in a batch (including order, insert and delete). */
  invalidate(root: any, ops: readonly PatchOp[]): void {
    for (const op of ops) {
      let node = root;
      if (node && typeof node === 'object') this.cache.delete(node);
      for (const part of op.path) {
        node = node?.[part];
        if (!node || typeof node !== 'object') break;
        this.cache.delete(node);
      }
    }
  }

  digest(root: any): string {
    this.hashed = this.reused = 0;
    return root === undefined ? '' : this.visit(root, 0);
  }
  private visit(value: any, depth: number): string {
    const container = value !== null && typeof value === 'object';
    const cached = container ? this.cache.get(value)?.get(depth) : undefined;
    if (cached !== undefined) { this.reused++; return cached; }
    this.hashed++;
    let digest: string;
    if (depth >= 8 || !container) digest = canonicalHash(value, 'v');
    else if (Array.isArray(value)) digest = canonicalHash(['a', value.map(child => this.visit(child, depth + 1))]);
    else digest = canonicalHash(['o', Object.keys(value).sort().map(key => [key, this.visit(value[key], depth + 1)])]);
    if (container) {
      let depths = this.cache.get(value);
      if (!depths) this.cache.set(value, depths = new Map());
      depths.set(depth, digest);
    }
    return digest;
  }
}
