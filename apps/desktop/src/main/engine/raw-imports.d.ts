/* Text imports Vite inlines at build time (`?raw`): the pinned Blender
   worker script (engine/crates/pm-blender/worker/worker.py). */
declare module '*.py?raw' {
  const text: string;
  export default text;
}
