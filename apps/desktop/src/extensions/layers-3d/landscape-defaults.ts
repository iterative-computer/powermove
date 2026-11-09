/** Content defaults only; the existing World, Terrain and Scatter controls edit them. */
export const BUILTIN_NATURE = [
    { id: 'nature:conifer', name: 'Pine' }, { id: 'nature:broadleaf', name: 'Oak' }, { id: 'nature:birch', name: 'Birch' },
    { id: 'nature:bush', name: 'Bush' }, { id: 'nature:grass', name: 'Grass' }, { id: 'nature:rock', name: 'Rock' }
];
export const LANDSCAPE_TERRAIN = {
    p: { width: 180, depth: 180, resolution: 128, adaptive: true, amplitude: 14, frequency: .018, octaves: 5, seed: 41, ridged: false, detail: .018, erosion: 12, natural: true, blend: .16, waterLevel: -9, snowLevel: 14 },
    regions: [{ p: { minSlope: 40, color: '#8C8272', roughness: .9 } }, { p: { minHeight: 14, maxSlope: 55, color: '#E9EEF0', roughness: .8 } }, { p: { maxHeight: -9, color: '#B7A078', roughness: .92 } }]
};
export const LANDSCAPE_WORLD = { mode: 'sky', preset: 'golden-hour', p: { elevation: 9, azimuth: -70, haze: 3, metersPerUnit: 1, fogDensity: .003, fogColor: '#E7D2B8', fogHeightFalloff: .08, fogAnisotropy: .55, fogDistance: 500 } };
export const LANDSCAPE_CAMERA = { p: { x: 10, y: 14, z: 62, targetX: -8, targetY: 2, targetZ: -35, fov: 41, far: 2000 } };
export const LANDSCAPE_SCATTERS = [
    { name: 'Pines', model: 'nature:conifer', p: { count: 360, seed: 517, scaleMin: .65, scaleMax: 1.05, maxSlope: 40 } },
    { name: 'Birches', model: 'nature:birch', p: { count: 130, seed: 643, scaleMin: .65, scaleMax: 1.05, maxSlope: 36 } },
    { name: 'Oaks', model: 'nature:broadleaf', p: { count: 30, seed: 91, scaleMin: .7, scaleMax: 1.1, maxSlope: 30 } },
    { name: 'Meadow', model: 'nature:grass', p: { count: 18000, seed: 633, scaleMin: .7, scaleMax: 1.5, maxSlope: 40 } },
    { name: 'Stones', model: 'nature:rock', p: { count: 60, seed: 483, scaleMin: .5, scaleMax: 1.7, maxSlope: 50 } },
    { name: 'Bushes', model: 'nature:bush', p: { count: 40, seed: 484, scaleMin: .5, scaleMax: 1.3, maxSlope: 40 } }
];
