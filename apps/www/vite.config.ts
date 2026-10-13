import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig, loadEnv } from 'vite';

export default defineConfig(({ command, mode }) => {
  if (command === 'build' && !loadEnv(mode, process.cwd(), 'VITE_').VITE_CF_BEACON_TOKEN) {
    console.warn('VITE_CF_BEACON_TOKEN is unset: building without Cloudflare Web Analytics.');
  }
  return { plugins: [sveltekit()] };
});
