// Extracted from v1.2.0 renderer/src/kernel/property-shortcuts.ts; local type/duplicate-helper paths adapted.
/** Timeline presentation only: these shortcuts never mutate animation data. */
export const propertyShortcuts = [
  ["p", "revealPos", "Position"],
  ["s", "revealScale", "Scale"],
  ["r", "revealRot", "Rotation"],
  ["t", "revealOpacity", "Opacity"],
  ["a", "revealAnchor", "Anchor point"],
  ["u", "revealKeys", "Animated properties"],
  ["m", "revealMasks", "Mask controls"],
  ["f", "revealFeather", "Mask feather"],
  ["e", "revealEffects", "Effects"],
  ["l", "revealAudio", "Audio levels"],
] as const;
