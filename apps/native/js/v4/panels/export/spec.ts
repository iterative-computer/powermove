/** Exact copy/field contract: design/panels/export-sheet.md §§1–3,8. */
export type Kind = "video" | "images" | "project";
export type Format = "prores" | "mp4" | "webm" | "png" | "still" | "json";
export const kinds = [
  { id: "video", label: "Video" },
  { id: "images", label: "Images" },
  { id: "project", label: "Project" },
];
export const descriptions: Record<Kind, string> = {
  video: "MP4, WebM or ProRes for delivery and editing.",
  images: "A single frame, or every frame as a PNG sequence.",
  project: "An editable .pmv with every layer and its media.",
};
export const formats: { v: Format; label: string }[] = [
  { v: "prores", label: "QuickTime · ProRes" },
  { v: "mp4", label: "MP4 · H.264 or HEVC" },
  { v: "webm", label: "WebM · VP9" },
  { v: "png", label: "PNG sequence" },
  { v: "still", label: "Still frame (PNG)" },
  { v: "json", label: "Project file (.pmv)" },
];
export const kindOf = (format: Format): Kind =>
  format === "json"
    ? "project"
    : format === "png" || format === "still"
      ? "images"
      : "video";
export type Profile = "422proxy" | "422lt" | "422" | "422hq" | "4444" | "4444xq";
export type Codec = "h264" | "hevc";
export const profiles: { v: Profile; label: string }[] = [
  { v: "422proxy", label: "ProRes 422 Proxy" },
  { v: "422lt", label: "ProRes 422 LT" },
  { v: "422", label: "ProRes 422" },
  { v: "422hq", label: "ProRes 422 HQ" },
  { v: "4444", label: "ProRes 4444" },
  { v: "4444xq", label: "ProRes 4444 XQ" },
];
export const codecs: { v: Codec; label: string }[] = [
  { v: "h264", label: "H.264" },
  { v: "hevc", label: "HEVC · 10-bit" },
];
export const depths = [
  { v: 8, label: "8-bit" },
  { v: 16, label: "16-bit" },
];
/** Settings saved before these fields existed mean today's output. */
export const fieldDefaults = { profile: "4444", codec: "h264", depth: 8 } as const;
export type Field =
  | "profile"
  | "codec"
  | "depth"
  | "scale"
  | "fps"
  | "range"
  | "quality"
  | "mblur"
  | "audio"
  | "alpha"
  | "useProxies";
export const fields: Record<Format, readonly Field[]> = {
  prores: ["profile", "scale", "fps", "range", "mblur", "audio", "alpha", "useProxies"],
  mp4: ["codec", "scale", "fps", "range", "quality", "mblur", "audio", "useProxies"],
  webm: ["scale", "fps", "range", "quality", "mblur", "audio", "useProxies"],
  png: ["depth", "scale", "fps", "range", "mblur", "alpha", "useProxies"],
  still: ["depth", "scale", "mblur", "alpha"],
  json: [],
};
/** Whether the sheet shows `field`: ProRes 422 has no alpha channel. */
export const shows = (
  o: { format: Format; profile?: string },
  field: Field,
) =>
  fields[o.format].includes(field) &&
  !(field === "alpha" && o.format === "prores" && !o.profile?.startsWith("4444"));
export const actions: Record<Format, string> = {
  prores: "Export Video",
  mp4: "Export Video",
  webm: "Export Video",
  png: "Export Frames",
  still: "Export Frame",
  json: "Save Project File",
};
export const extensions: Record<Format, string> = {
  prores: "mov",
  mp4: "mp4",
  webm: "webm",
  png: "png",
  still: "png",
  json: "pmv",
};
export const rates = [
  { v: 24000 / 1001, label: "23.976 fps" },
  { v: 24, label: "24 fps" },
  { v: 25, label: "25 fps" },
  { v: 30000 / 1001, label: "29.97 fps" },
  { v: 30, label: "30 fps" },
  { v: 50, label: "50 fps" },
  { v: 60000 / 1001, label: "59.94 fps" },
  { v: 60, label: "60 fps" },
];
export const dynamicRanges = [
  { v: "sdr", label: "SDR" },
  { v: "pq", label: "HDR · PQ" },
  { v: "hlg", label: "HDR · HLG" },
];
export const qualities = [
  { v: "draft", label: "Draft · 4 Mbps" },
  { v: "high", label: "High · 16 Mbps" },
  { v: "max", label: "Max · 40 Mbps" },
];
/** Per-export 3D quality; the first row is the composition's own preset. */
export const studioQualities = (saved: string) => [
  { v: "composition", label: `Composition · ${saved}` },
  { v: "draft", label: "Draft", detail: "Check motion and timing" },
  { v: "preview", label: "Preview", detail: "A quicker look for review" },
  { v: "production", label: "Production", detail: "Clean, final-looking" },
  { v: "final", label: "Final", detail: "Reference quality" },
];
/** What each 3D quality promises (owner decision 2026-11-11): the row's
 * second line names the chosen one's speed and look in plain words. */
export const studioPromises: Record<string, string> = {
  draft:
    "A quick check of motion and timing, about 1 s a frame. It can look soft.",
  preview: "A quicker, cleaner look for review. Not for delivery.",
  production:
    "A clean, final-looking picture: about 3 s a frame, up to 8 s with glass or chrome.",
  final: "Reference quality. It takes as long as it needs.",
  custom: "The composition's own sampling settings.",
};
/** Added to the Quality row when this build has no Open Image Denoise. */
export const guidedDenoiseNote =
  "This build has no Open Image Denoise, so the built-in filter cleans the frames.";
/** The Render Time row's second line, by where the estimate comes from. */
export const estimateNotes = {
  measured: "Measured on this Mac for this scene and size.",
  measuring: "Measuring on this Mac…",
  guess: "A rough guess until this scene is measured.",
} as const;
export const sidecars = [
  { v: "none", label: "None" },
  { v: "srt", label: "SubRip (.srt)" },
  { v: "vtt", label: "WebVTT (.vtt)" },
];
export const rows = {
  dynamicRange: [
    "Dynamic Range",
    "HDR keeps the project's light. SDR tone maps it to SDR white.",
  ],
  format: ["Format", "Container and codec of the exported file."],
  profile: [
    "Profile",
    "422 HQ for editing and delivery, 4444 to keep transparency.",
  ],
  codec: ["Codec", "HEVC is 10-bit and smaller than H.264 at the same quality."],
  depth: ["Depth", "16-bit keeps smooth gradients for grading and compositing."],
  scale: [
    "Resolution",
    "Rendered size relative to the composition, or any pixel size.",
  ],
  size: ["Size", "Exact pixel size to render; keeps the composition aspect."],
  fps: ["Frame Rate", "Frames written per second of exported video."],
  range: ["Range", "Export the work area or the whole composition."],
  quality: ["Quality", "Video bitrate used for video exports."],
  mblur: ["Motion Blur", "Render motion blur for moving layers."],
  studioQuality: [
    "3D Quality",
    "Cleaner 3D takes longer per frame.",
  ],
  studioTime: ["Render Time", "Measured on this Mac for this scene and size."],
  studioReducedSize: [
    "Render at 75% Size",
    "Faster, slightly softer. Full size is always the default.",
  ],
  useProxies: [
    "Use Proxies (Draft)",
    "Read proxy media where it exists, for a quick draft. Otherwise exports always read originals.",
  ],
  audio: ["Include Audio", "Mix composition audio into the video."],
  alpha: [
    "Transparent Background",
    "Keep the background see-through, with straight alpha.",
  ],
  captionsBurn: ["Burn In Captions", "Draw captions into the picture."],
  captionsSidecar: [
    "Caption File",
    "Also write a subtitle file beside the video.",
  ],
  preset: ["Saved Preset", "Load a set of export settings you saved earlier."],
  presetSave: [
    "Save as Preset",
    "Name these settings to reuse them in any project.",
  ],
} as const;
export const sizeError = "Enter a size from 16 to 8192 px.";
export const colorNote =
  "Color: sRGB. ProRes uses tagged BT.709 primaries and sRGB transfer. Transparent output uses straight alpha.";
