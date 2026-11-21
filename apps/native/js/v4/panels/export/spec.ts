/** Exact copy/field contract: design/panels/export-sheet.md §§1–3,8. */
export type Kind = "video" | "images" | "project";
export type Format = "prores" | "mp4" | "webm" | "png" | "still" | "json";
export const kinds = [
  { id: "video", label: "Video" },
  { id: "images", label: "Images" },
  { id: "project", label: "Project" },
];
export const descriptions: Record<Kind, string> = {
  video: "QuickTime, MP4 or WebM for delivery and editing.",
  images: "A single frame, or every frame as a PNG sequence.",
  project: "An editable .pmv with every layer and its media.",
};
export const formats: { v: Format; label: string }[] = [
  { v: "prores", label: "QuickTime (.mov)" },
  { v: "mp4", label: "MP4" },
  { v: "webm", label: "WebM" },
  { v: "png", label: "PNG Sequence" },
  { v: "still", label: "Still Frame (PNG)" },
  { v: "json", label: "Project File (.pmv)" },
];
export const kindOf = (format: Format): Kind =>
  format === "json"
    ? "project"
    : format === "png" || format === "still"
      ? "images"
      : "video";
export type Profile =
  | "422proxy"
  | "422lt"
  | "422"
  | "422hq"
  | "4444"
  | "4444xq";
export type Codec = "h264" | "hevc";
export type VideoCodec = "prores" | "h264" | "hevc" | "hevc10" | "vp9" | "vp8";
export type AlphaMode = "none" | "straight" | "premultiplied";
export const videoCodecs: { v: VideoCodec; label: string }[] = [
  { v: "prores", label: "ProRes" },
  { v: "h264", label: "H.264 (High)" },
  { v: "hevc", label: "HEVC (Main)" },
  { v: "hevc10", label: "HEVC 10-bit (Main10)" },
  { v: "vp9", label: "VP9" },
  { v: "vp8", label: "VP8" },
];
type Selection = {
  format: Format;
  profile?: string;
  codec?: Codec;
  videoCodec?: VideoCodec;
  alpha?: boolean;
  alphaMode?: AlphaMode;
  dynamicRange?: string;
};
export const codecOptions = (o: Selection) =>
  videoCodecs.filter(
    ({ v }) =>
      (o.format === "prores"
        ? ["prores", "h264", "hevc", "hevc10"]
        : o.format === "mp4"
          ? ["h264", "hevc", "hevc10"]
          : o.format === "webm"
            ? ["vp9", "vp8"]
            : []
      ).includes(v) &&
      (!(
        o.dynamicRange &&
        o.dynamicRange !== "sdr" &&
        ["prores", "mp4"].includes(o.format)
      ) ||
        ["prores", "hevc10"].includes(v)),
  );
export function selectedCodec(o: Selection): VideoCodec {
  const codec =
    o.videoCodec ??
    (o.format === "prores"
      ? "prores"
      : o.format === "webm"
        ? "vp9"
        : o.codec === "hevc"
          ? "hevc10"
          : "h264");
  return o.dynamicRange &&
    o.dynamicRange !== "sdr" &&
    ["prores", "mp4"].includes(o.format) &&
    codec !== "prores"
    ? "hevc10"
    : codec;
}
export function alphaOptions(o: Selection): { v: AlphaMode; label: string }[] {
  const codec = selectedCodec(o);
  const image = o.format === "png" || o.format === "still";
  const straight =
    image ||
    (kindOf(o.format) === "video" &&
      (codec === "prores"
        ? (o.profile ?? "4444").startsWith("4444")
        : ["hevc", "vp9", "vp8"].includes(codec)));
  return [
    { v: "none", label: "None" },
    ...(straight ? [{ v: "straight" as const, label: "Straight" }] : []),
    ...(straight && !image && ["prores", "hevc"].includes(codec)
      ? [{ v: "premultiplied" as const, label: "Premultiplied" }]
      : []),
  ];
}
export function selectedAlpha(o: Selection): AlphaMode {
  if (o.alphaMode) return o.alphaMode;
  return o.alpha &&
    ["prores", "png", "still"].includes(o.format) &&
    alphaOptions(o).length > 1
    ? "straight"
    : "none";
}
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
export type ColorTag = "rec709QuickTime" | "rec709";
export type AudioCoding = "pcm24" | "pcm16" | "pcmFloat" | "aac" | "opus";
/** The label an SDR video carries; the picture is the same codes either way. */
export const colorTags: { v: ColorTag; label: string }[] = [
  { v: "rec709QuickTime", label: "Rec.709 (QuickTime and Safari)" },
  { v: "rec709", label: "Rec.709" },
];
export const audioCodings: { v: AudioCoding; label: string }[] = [
  { v: "pcm24", label: "24-bit PCM" },
  { v: "pcm16", label: "16-bit PCM" },
  { v: "pcmFloat", label: "32-bit float PCM" },
  { v: "aac", label: "AAC" },
  { v: "opus", label: "Opus" },
];
export const audioOptions = (o: { format: Format }) =>
  audioCodings.filter(({ v }) =>
    o.format === "prores"
      ? v !== "opus"
      : v === (o.format === "webm" ? "opus" : "aac"),
  );
export const selectedAudio = (o: {
  format: Format;
  audioCoding?: AudioCoding;
}) =>
  audioOptions(o).find(({ v }) => v === o.audioCoding)?.v ??
  (o.format === "prores" ? "pcm24" : o.format === "webm" ? "opus" : "aac");
/** Settings saved before these fields existed mean today's output. */
export const fieldDefaults = {
  profile: "4444",
  codec: "h264",
  depth: 8,
  colorTag: "rec709QuickTime",
  timecode: "",
  audioCoding: "pcm24",
} as const;
export type Field =
  | "profile"
  | "codec"
  | "videoCodec"
  | "alphaMode"
  | "depth"
  | "scale"
  | "fps"
  | "range"
  | "quality"
  | "mblur"
  | "audio"
  | "alpha"
  | "useProxies"
  | "colorTag"
  | "timecode"
  | "timecodeTrack"
  | "audioCoding";
export const fields: Record<Format, readonly Field[]> = {
  prores: [
    "videoCodec",
    "profile",
    "scale",
    "fps",
    "range",
    "quality",
    "colorTag",
    "timecodeTrack",
    "timecode",
    "mblur",
    "audio",
    "audioCoding",
    "alphaMode",
    "useProxies",
  ],
  mp4: [
    "videoCodec",
    "scale",
    "fps",
    "range",
    "quality",
    "colorTag",
    "mblur",
    "audio",
    "audioCoding",
    "alphaMode",
    "useProxies",
  ],
  webm: [
    "videoCodec",
    "scale",
    "fps",
    "range",
    "quality",
    "mblur",
    "audio",
    "audioCoding",
    "alphaMode",
    "useProxies",
  ],
  png: ["depth", "scale", "fps", "range", "mblur", "alphaMode", "useProxies"],
  still: ["depth", "scale", "mblur", "alphaMode"],
  json: [],
};
/** Whether the sheet shows `field`: ProRes 422 has no alpha channel. */
export const shows = (o: Selection, field: Field) => {
  if (field === "videoCodec" || field === "codec")
    return kindOf(o.format) === "video";
  if (field === "profile")
    return o.format === "prores" && selectedCodec(o) === "prores";
  if (field === "alpha" || field === "alphaMode")
    return alphaOptions(o).length > 1;
  if (field === "audioCoding") return audioOptions(o).length > 1;
  if (field === "quality")
    return (
      kindOf(o.format) === "video" &&
      ["h264", "hevc", "hevc10", "vp8", "vp9"].includes(selectedCodec(o))
    );
  return fields[o.format].includes(field);
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
/** The listed rate a stored rate is: a project keeps 29.97 as that float,
 * and the engine exports any rate within 0.02 % of a listed one as it. */
export function listedRate(fps: number): number {
  return rates.find((r) => Math.abs(fps - r.v) <= 0.0002 * r.v)?.v ?? fps;
}
/** The Frame Rate choices for an export at `fps`: the list, led by the
 * export's own rate when it is none of them (a 12 fps composition). */
export function rateOptions(fps: number) {
  const own = listedRate(fps);
  return rates.some((r) => r.v === own)
    ? rates
    : [{ v: own, label: `${+own.toFixed(3)} fps` }, ...rates];
}
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
/** The name the sheet gives the HLG HEVC metadata macOS adds: factual
 * wording only, no mark or logo, in one place so it can change. */
export const dolbyVisionLabel = "Dolby Vision 8.4";
export const rows = {
  dolbyVision: [
    dolbyVisionLabel,
    "Metadata macOS adds to HLG HEVC for displays that use it. Players without it show the HLG picture.",
  ],
  dynamicRange: [
    "Dynamic Range",
    "HDR keeps the project's light. SDR tone maps it to SDR white.",
  ],
  format: ["Format", "Container and codec of the exported file."],
  profile: [
    "Profile",
    "422 HQ for editing and delivery, 4444 to keep transparency.",
  ],
  codec: ["Codec", "Video coding and bit depth of the exported file."],
  alphaMode: ["Alpha", "How transparency is stored in the exported picture."],
  timecodeTrack: [
    "Timecode Track",
    "Write the composition's timecode, or a custom start timecode, in the QuickTime movie.",
  ],
  depth: [
    "Depth",
    "16-bit keeps smooth gradients for grading and compositing.",
  ],
  scale: [
    "Resolution",
    "Rendered size relative to the composition, or any pixel size.",
  ],
  size: ["Size", "Exact pixel size to render; keeps the composition aspect."],
  fps: ["Frame Rate", "Frames written per second of exported video."],
  range: ["Range", "Export the work area or the whole composition."],
  quality: ["Quality", "Video bitrate used for video exports."],
  mblur: ["Motion Blur", "Render motion blur for moving layers."],
  studioQuality: ["3D Quality", "Cleaner 3D takes longer per frame."],
  studioTime: ["Render Time", "Measured on this Mac for this scene and size."],
  studioReducedSize: [
    "Render at 75% Size",
    "Faster, slightly softer. Full size is always the default.",
  ],
  useProxies: [
    "Use Proxies (Draft)",
    "Read proxy media where it exists, for a quick draft. Otherwise exports always read originals.",
  ],
  colorTag: [
    "Color Tag",
    "How the file labels its color. The picture is the same either way.",
  ],
  timecode: [
    "Start Timecode",
    "Timecode of the first frame, as HH:MM:SS:FF, or ;FF for drop-frame. Leave empty to use the composition's time.",
  ],
  audio: ["Include Audio", "Mix composition audio into the video."],
  audioCoding: ["Audio Format", "Sample format of the audio track."],
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
/** With the Rec.709 label chosen for an SDR ProRes or MP4. */
export const colorNoteRec709 =
  "Color: Rec.709. The file is tagged BT.709 primaries and BT.709 transfer.";
export const colorNote =
  "Color: sRGB. Video uses tagged BT.709 primaries and sRGB transfer.";
