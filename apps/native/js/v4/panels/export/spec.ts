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
  { v: "prores", label: "QuickTime · ProRes 4444" },
  { v: "mp4", label: "MP4 · H.264" },
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
export type Field =
  | "scale"
  | "fps"
  | "range"
  | "quality"
  | "mblur"
  | "audio"
  | "alpha";
export const fields: Record<Format, readonly Field[]> = {
  prores: ["scale", "fps", "range", "mblur", "audio", "alpha"],
  mp4: ["scale", "fps", "range", "quality", "mblur", "audio"],
  webm: ["scale", "fps", "range", "quality", "mblur", "audio"],
  png: ["scale", "fps", "range", "mblur", "alpha"],
  still: ["scale", "mblur", "alpha"],
  json: [],
};
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
export const qualities = [
  { v: "draft", label: "Draft · 4 Mbps" },
  { v: "high", label: "High · 16 Mbps" },
  { v: "max", label: "Max · 40 Mbps" },
];
export const sidecars = [
  { v: "none", label: "None" },
  { v: "srt", label: "SubRip (.srt)" },
  { v: "vtt", label: "WebVTT (.vtt)" },
];
export const rows = {
  format: ["Format", "Container and codec of the exported file."],
  scale: [
    "Resolution",
    "Rendered size relative to the composition, or any pixel size.",
  ],
  size: ["Size", "Exact pixel size to render; keeps the composition aspect."],
  fps: ["Frame Rate", "Frames written per second of exported video."],
  range: ["Range", "Export the work area or the whole composition."],
  quality: ["Quality", "Video bitrate used for video exports."],
  mblur: ["Motion Blur", "Render motion blur for moving layers."],
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
