import {it,expect} from 'vitest';
import {validMatteSource,validMatteSources} from './matte';
it('allows group mattes and acyclic intersections while rejecting all matte/group cycles',()=>{
 const a={id:'a',type:'shape'},b={id:'b',type:'shape'},g={id:'g',type:'group'},child={id:'child',type:'shape',group:'g'},layers=[a,b,g,child];
 expect(validMatteSource(layers,a,'g')).toBe(true);expect(validMatteSource(layers,child,'g')).toBe(false);
 expect(validMatteSources(layers,a,[{source:'b',mode:'luma'},{source:'g',mode:'alpha'}])).toBe(true);
 expect(validMatteSources([...layers,{id:'cyclic',type:'shape',matteSources:[{source:'a'}]}],a,[{source:'cyclic'}])).toBe(false);
 expect(validMatteSource(layers,a,'missing')).toBe(false);expect(validMatteSource([...layers,{id:'audio',type:'audio'}],a,'audio')).toBe(false);
});
