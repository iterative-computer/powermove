/* Small 24×24 line icons for the 3D viewport header and toolbar, drawn with currentColor. */
const stroke='fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"';
export const ICONS={
  tweak:`<path d="M6 3.5v14l3.8-3.6 2.6 6 2.4-1-2.6-5.9 5.3-.2z" ${stroke}/>`,
  box:`<rect x="3.5" y="3.5" width="13" height="11" rx="1" ${stroke} stroke-dasharray="2.6 2.4"/><path d="M12 11v10l2.7-2.5 1.8 4 1.7-.8-1.8-4 3.7-.1z" ${stroke}/>`,
  cursor:`<circle cx="12" cy="12" r="5.5" ${stroke} stroke-dasharray="3 2.8"/><path d="M12 2.5v5M12 16.5v5M2.5 12h5M16.5 12h5" ${stroke}/>`,
  move:`<path d="M12 3v18M3 12h18M9 6l3-3 3 3M9 18l3 3 3-3M6 9l-3 3 3 3M18 9l3 3-3 3" ${stroke}/>`,
  rotate:`<path d="M19.5 12a7.5 7.5 0 1 1-2.2-5.3" ${stroke}/><path d="M19.8 3.8v3.6h-3.6" ${stroke}/>`,
  scale:`<rect x="3.5" y="9.5" width="11" height="11" rx="1" ${stroke}/><path d="M13 3.5h7.5V11M20.5 3.5 12 12" ${stroke}/>`,
  wireframe:`<circle cx="12" cy="12" r="8" ${stroke}/><ellipse cx="12" cy="12" rx="3.6" ry="8" ${stroke}/><path d="M4 12h16" ${stroke}/>`,
  solid:`<circle cx="12" cy="12" r="8" fill="currentColor"/>`,
  material:`<circle cx="12" cy="12" r="8" ${stroke}/><path d="M12 4a8 8 0 0 0 0 16z" fill="currentColor" opacity=".55"/><circle cx="9" cy="9" r="1.8" fill="currentColor"/>`,
  rendered:`<circle cx="12" cy="12" r="8" ${stroke}/><path d="M12 4a8 8 0 0 1 0 16 5.5 8 0 0 0 0-16z" fill="currentColor"/>`,
  xray:`<rect x="4" y="6" width="11" height="11" rx="1" ${stroke} opacity=".55"/><rect x="9" y="3" width="11" height="11" rx="1" ${stroke}/>`,
  overlays:`<circle cx="9.5" cy="12" r="5.5" ${stroke}/><circle cx="14.5" cy="12" r="5.5" ${stroke}/>`,
  magnet:`<path d="M6 4v8a6 6 0 0 0 12 0V4h-3.5v8a2.5 2.5 0 0 1-5 0V4z" ${stroke}/><path d="M6 7.5h3.5M14.5 7.5H18" ${stroke}/>`,
  orientation:`<path d="M5 19V6M5 19h13M5 19l8-8" ${stroke}/><path d="m3 8 2-2.5L7 8M16 17l2.5 2-2.5 2" ${stroke}/>`,
  median:`<circle cx="12" cy="12" r="7.5" ${stroke}/><circle cx="12" cy="12" r="2" fill="currentColor"/>`,
  individual:`<circle cx="7" cy="8" r="3.5" ${stroke}/><circle cx="17" cy="16" r="3.5" ${stroke}/><circle cx="7" cy="8" r="1" fill="currentColor"/><circle cx="17" cy="16" r="1" fill="currentColor"/>`,
  bounds:`<rect x="4" y="4" width="16" height="16" ${stroke} stroke-dasharray="3 2.5"/><circle cx="12" cy="12" r="1.8" fill="currentColor"/>`,
  active:`<circle cx="8" cy="15" r="3.5" ${stroke}/><circle cx="16" cy="9" r="3.5" fill="currentColor"/>`,
  record:`<circle cx="12" cy="12" r="6" fill="currentColor"/>`,
  lock:`<rect x="5" y="10.5" width="14" height="10" rx="2" ${stroke}/><path d="M8.5 10.5V8a3.5 3.5 0 0 1 7 0v2.5" ${stroke}/>`,
  chevron:`<path d="m7 10 5 5 5-5" ${stroke}/>`
} as const;
export type IconName=keyof typeof ICONS;
export const icon=(name:IconName,size=16)=>`<svg viewBox="0 0 24 24" width="${size}" height="${size}" aria-hidden="true">${ICONS[name]}</svg>`;
