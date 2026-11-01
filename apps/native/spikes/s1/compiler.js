import {compile} from 'svelte/compiler';
const result=compile('<script>let count = $state(0)</script><button onclick={() => count++}>{count}</button>',{filename:'Synthetic.svelte',generate:'client'});
if(!result.js.code.includes('count'))throw Error('Svelte compile failed');
globalThis.probeResult={probe:'svelte-compiler-isolate',version:'5.56.10',js_bytes:result.js.code.length,warnings:result.warnings.length};
