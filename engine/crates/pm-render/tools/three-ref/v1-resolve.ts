import { plugin } from 'bun';
import { resolve } from 'node:path';

// The Electron app's tree (three.js r180 and the production SceneRuntime) ships
// from main, not this branch. These reference tools render with it, so they read
// it from a v1 checkout: point PM_V1_REF at it (default /tmp/pm-ref-v1.2.0).
export const v1Root = resolve(process.env.PM_V1_REF ?? '/tmp/pm-ref-v1.2.0');

/** Redirects `apps/desktop/...` import specifiers (relative or bare) to the v1 checkout. */
export const v1Resolve = {
  name: 'v1-desktop',
  setup(build: { onResolve: (o: { filter: RegExp }, cb: (a: { path: string }) => { path: string }) => void }) {
    build.onResolve({ filter: /apps\/desktop\// }, (args) => ({
      path: resolve(v1Root, args.path.slice(args.path.indexOf('apps/desktop/'))),
    }));
  },
};

/** Registers the redirect for this process's runtime imports. Call before importing modules that reach into apps/desktop. */
export function registerV1Resolve(): void {
  plugin(v1Resolve);
}
