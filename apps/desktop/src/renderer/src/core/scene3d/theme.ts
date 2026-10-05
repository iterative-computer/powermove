import * as THREE from 'three';
/** Resolve a theme token through computed style; color-mix() resolves to srgb. */
export function themeColor(element:HTMLElement,token:string,fallback:number):THREE.Color {
  const probe=document.createElement('span');probe.style.cssText=`position:absolute;visibility:hidden;color:var(${token})`;
  element.append(probe);const value=getComputedStyle(probe).color;probe.remove();
  const color=new THREE.Color(fallback),channels=value.match(/-?[\d.]+/g)?.map(Number);
  if(!channels||channels.length<3||!/^(?:rgba?\(|color\(srgb)/.test(value))return color;
  const scale=value.startsWith('color(')?1:255;
  return color.setRGB(channels[0]!/scale,channels[1]!/scale,channels[2]!/scale,THREE.SRGBColorSpace);
}
