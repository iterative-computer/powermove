import { expect, test } from 'vitest';
import { layerDrop } from './layer-drop';
const rows = [
  {kind:'layer',L:{id:'g',type:'group',name:'Group'},depth:0},
  {kind:'layer',L:{id:'c',group:'g'},depth:1},
  {kind:'prop',L:{id:'c',group:'g'},depth:1},
  {kind:'layer',L:{id:'b'},depth:0},
];
test('group center nests, edges place outside the entire group',()=>{
  expect(layerDrop(rows,.5,110)).toMatchObject({mode:'inside',group:'g'});
  expect(layerDrop(rows,.1,110)).toMatchObject({mode:'before',group:null,row:0});
  expect(layerDrop(rows,.9,110)).toMatchObject({mode:'after',group:null,row:3});
  expect(layerDrop(rows,.5,60)).toMatchObject({mode:'after',group:null});
});
test('properties resolve to their owner and child gaps preserve membership',()=>{
  expect(layerDrop(rows,2.5,110)).toMatchObject({target:'c',mode:'after',group:'g',row:3});
  expect(layerDrop(rows,1.1,110)).toMatchObject({mode:'before',group:'g',depth:1});
});
