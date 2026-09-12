// Single seam for the hero's scene player. The SVG adapter is site-owned (it
// renders the glow halo the desktop SVG backend does not); the animation engine
// itself comes from the shared @powermove/player bundle.
export { createSvgPlayer } from './svg-player';
