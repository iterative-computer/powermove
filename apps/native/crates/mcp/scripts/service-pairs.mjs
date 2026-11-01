// Execute frozen validators and Store gateway bodies over synthetic services.
import { tag, evaluate } from './tag-oracle.mjs';
import fs from 'node:fs';
const input=JSON.parse(fs.readFileSync(0,'utf8'));
let result;
try {
  if(input.tool==='validate_effect') {
    const {validateEffect}=evaluate(tag('apps/desktop/src/renderer/src/kernel/glsl.ts'),['validateEffect']);
    const d=validateEffect(input.arguments.definition);
    result={valid:true,id:d.id,parameterCount:d.params.length,note:'Definition validation only. This does not register the effect or compile/render its shader.'};
  } else {
    const {createStoreAgentGateway}=evaluate(tag('apps/desktop/src/main/cloud/store-agent.ts'),['createStoreAgentGateway']);
    const store={extensions:async()=>input.page,detail:async()=>input.detail,tree:async()=>input.tree,release:async()=>input.release,file:async()=>input.text};
    const gateway=createStoreAgentGateway({store});
    if(input.tool==='store_search')result=await gateway.search(input.arguments);
    else if(input.tool==='store_extension')result=await gateway.detail(input.arguments.handle,input.arguments.slug);
    else if(input.tool==='store_source')result=await gateway.source(input.arguments.releaseId,input.arguments.path);
    else throw Error('Unsupported service oracle');
  }
  process.stdout.write(JSON.stringify({result}));
} catch(e) {process.stdout.write(JSON.stringify({error:e.message}));}
