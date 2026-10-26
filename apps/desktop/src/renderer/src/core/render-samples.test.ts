import {expect,it} from 'vitest';
import {motionBlurTimes} from './render-samples';
it('matches shutter sample centers and negative-shutter ordering exactly',()=>{
 expect(motionBlurTimes(.5,4,{mbSamples:4,shutter:1})).toEqual([.40625,.46875,.53125,.59375]);
 expect(motionBlurTimes(.5,4,{mbSamples:4,shutter:-1})).toEqual([.59375,.53125,.46875,.40625]);
 expect(motionBlurTimes(.5,4,{mbSamples:0})).toHaveLength(10);
 expect(motionBlurTimes(.5,4,{mbSamples:10000})).toHaveLength(64);
});
