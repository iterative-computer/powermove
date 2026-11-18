// bun engine/crates/pm-render/tests/fixtures/generate-extension-effects.ts
import { definitionSnapshots } from '../../../../../apps/native/js/compat/src/definitions';
import { definition as corner } from '../../../../../apps/native/js/compat/api-pack/samples/corner-pin-snap/index';
import { definition as tint } from '../../../../../apps/native/js/compat/api-pack/samples/gradient-tint/index';
const effects = [corner,tint];
const snapshots = definitionSnapshots({
  effects: {entries:()=>effects.map(item=>({id:item.id,item,ownerId:'sample'}))},
  transitions:{entries:()=>[]},layerTypes:{entries:()=>[]}
});
await Bun.write(import.meta.dir+'/extension-effects.json',JSON.stringify(snapshots,null,2)+'\n');
