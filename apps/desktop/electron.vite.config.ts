import { defineConfig } from 'electron-vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import path from 'node:path';
import { playerBundlePlugin } from './scripts/player-bundle';

export default defineConfig({
  // A sandboxed preload must be CommonJS; pin the format so a future
  // `"type": "module"` in package.json cannot silently flip it to .mjs.
  main: {
    build: {
      outDir: 'out/main',
      rollupOptions: { input: { index: path.resolve(__dirname, 'src/main/entry.ts'), editor: path.resolve(__dirname, 'src/main/index.ts') }, output: { format: 'cjs', entryFileNames: '[name].js' } }
    }
  },
  preload: {
    build: {
      outDir: 'out/preload',
      rollupOptions: {
        input: {
          index: path.resolve(__dirname, 'src/preload/index.ts'),
          onboarding: path.resolve(__dirname, 'src/preload/onboarding.ts')
        },
        output: { format: 'cjs', entryFileNames: '[name].js' }
      }
    }
  },
  renderer: {
    root: 'src/renderer',
    publicDir: 'public',
    plugins: [svelte(), playerBundlePlugin()],
    resolve: { alias: { powermove: path.resolve(__dirname, 'src/renderer/src/kernel/api.ts') } },
    build: {
      outDir: 'out/renderer',
      rollupOptions: {
        input: {
          index: path.resolve(__dirname, 'src/renderer/index.html'),
          onboardingWelcome: path.resolve(__dirname, 'src/renderer/onboarding/welcome.html')
        },
        output: {
          assetFileNames: (assetInfo) => {
            const name = assetInfo.names[0] ?? '';
            return /\.(?:otf|ttf|woff2?)$/i.test(name)
              ? 'assets/fonts/[name]-[hash][extname]'
              : 'assets/[name]-[hash][extname]';
          }
        }
      }
    }
  }
});
