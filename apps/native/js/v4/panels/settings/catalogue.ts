import type { Json } from "../../src/protocol";
import type { Option } from "../../src/components";

export const sections = [
  "project",
  "memory",
  "general",
  "media",
  "accounts",
  "transcription",
  "extensions",
] as const;
export type Section = (typeof sections)[number];
export const sectionCopy: Record<
  Section,
  { title: string; description: string; symbol: string }
> = {
  project: { title: "Project", description: "", symbol: "doc.richtext" },
  memory: {
    title: "Memory",
    description:
      "Notes agents keep about this project: what the video is for, and why it looks the way it does.",
    symbol: "list.bullet",
  },
  general: {
    title: "General",
    description: "How Powermove looks and works on this Mac.",
    symbol: "gearshape",
  },
  media: {
    title: "Media",
    description:
      "Proxies, optimized media, and where Powermove keeps them on this Mac.",
    symbol: "film.stack",
  },
  accounts: {
    title: "Accounts",
    description:
      "Your Powermove account, and the models your agent works with.",
    symbol: "person.crop.circle",
  },
  transcription: {
    title: "Transcription",
    description:
      "Turn speech into text for captions and your agent, on this Mac.",
    symbol: "waveform.and.mic",
  },
  extensions: {
    title: "Extensions",
    description: "Manage extensions and turn them on or off.",
    symbol: "puzzlepiece.extension",
  },
};
export type Control =
  | "text"
  | "number"
  | "choice"
  | "switch"
  | "secure"
  | "colour"
  | "action"
  | "status";
export interface Row {
  id: string;
  section: Section;
  group: string;
  label: string;
  detail?: string;
  control: Control;
  value?: Json;
  options?: Option[];
  unit?: string;
  min?: number;
  max?: number;
  disabled?: boolean;
  busy?: boolean;
  error?: string;
  action?: string;
  actionLabel?: string;
  profileKey?: string;
  // Native Bindings leases are supplied by the host, never invented by a panel.
  binding?: string;
}
const choices = (values: (string | [Json, string])[]): Option[] =>
  values.map((v) =>
    typeof v === "string" ? { v, label: v } : { v: v[0], label: v[1] },
  );
export function generalRows(multiWindow: boolean): Row[] {
  return [
    {
      id: "appearance",
      section: "general",
      group: "Appearance",
      label: "Appearance",
      control: "choice",
      value: "system",
      profileKey: "themeMode",
      options: choices([
        ["system", "Match System"],
        ["light", "Light"],
        ["dark", "Dark"],
      ]),
      detail: "Follow your system setting, or pick a light or dark theme.",
    },
    {
      id: "workspace",
      section: "general",
      group: "Workspace",
      label: "After Effects Workspace",
      control: "action",
      action: "workspace.import",
      actionLabel: "Bring My Workspace…",
      detail:
        "Bring your saved panels and tools into Powermove with your agent.",
    },
    {
      id: "preview-memory",
      section: "general",
      group: "Playback",
      label: "Preview Memory Limit",
      control: "choice",
      value: 256,
      profileKey: "previewMemoryMiB",
      options: choices([
        [0, "Off"],
        [128, "128 MB"],
        [256, "256 MB"],
        [512, "512 MB"],
        [1024, "1 GB"],
        [2048, "2 GB"],
      ]),
      detail: "Frames kept in memory so replays are instant.",
    },
    {
      id: "cloud-media",
      section: "general",
      group: "Media",
      label: "Automatically Download Cloud Media",
      control: "switch",
      value: false,
      profileKey: "autoDownloadCloudMedia",
      detail: "Download offloaded files when opening a project.",
    },
    ...(multiWindow
      ? [
          {
            id: "restore-windows",
            section: "general" as const,
            group: "Windows",
            label: "Reopen Last Project on Launch",
            control: "switch" as const,
            value: true,
            profileKey: "restoreWindows",
            detail: "Open only the project you were last working on.",
          },
        ]
      : []),
  ];
}

/** Neutral disabled rows until a typed project/service adapter publishes values. */
export function projectRows(): Row[] {
  const row = (
    id: string,
    group: string,
    label: string,
    control: Control,
    detail?: string,
  ): Row => ({
    id: `project.${id}`,
    section: "project",
    group,
    label,
    control,
    detail,
    disabled: true,
  });
  return [
    row(
      "name",
      "Project",
      "Name",
      "text",
      "Shown in the title bar and used for exported file names.",
    ),
    {
      ...row(
        "size",
        "Composition",
        "Size",
        "choice",
        "Pick a common size, or set your own pixel dimensions.",
      ),
      // Shipped RESOLUTION_PRESETS order (v1.2.0 project-settings.ts), then Custom….
      options: choices([
        "4K UHD · 3840×2160",
        "HD 1080p · 1920×1080",
        "HD 720p · 1280×720",
        "Vertical HD · 1080×1920",
        "Portrait 4:5 · 1080×1350",
        "Square · 1080×1080",
        "Custom…",
      ]),
    },
    {
      ...row(
        "fps",
        "Composition",
        "Frame Rate",
        "number",
        "Frames rendered per second of the composition — any rate from 1 to 240.",
      ),
      min: 1,
      max: 240,
      unit: "fps",
    },
    { ...row("duration", "Composition", "Duration", "number"), unit: "s" },
    row(
      "background",
      "Composition",
      "Background",
      "colour",
      "Colour shown behind every layer of the composition.",
    ),
    {
      ...row(
        "depth",
        "Composition",
        "Color Depth",
        "choice",
        "Precision per color channel. Applies to every composition in this project.",
      ),
      options: choices(["8-bit", "16-bit", "32-bit float"]),
    },
    row(
      "format",
      "Export defaults",
      "Format",
      "choice",
      "Container and codec of the exported file.",
    ),
    row(
      "resolution",
      "Export defaults",
      "Resolution",
      "choice",
      "Rendered size relative to the composition, or any pixel size.",
    ),
    row(
      "export-fps",
      "Export defaults",
      "Frame Rate",
      "choice",
      "Frames written per second of exported video.",
    ),
    row(
      "range",
      "Export defaults",
      "Range",
      "choice",
      "Export the work area or the whole composition.",
    ),
    row(
      "quality",
      "Export defaults",
      "Quality",
      "choice",
      "Video bitrate used for video exports.",
    ),
    row(
      "motion-blur",
      "Export defaults",
      "Motion Blur",
      "switch",
      "Render motion blur for moving layers.",
    ),
    row(
      "audio",
      "Export defaults",
      "Include Audio",
      "switch",
      "Mix composition audio into the video.",
    ),
    row(
      "alpha",
      "Export defaults",
      "Transparent Background",
      "switch",
      "Keep the background see-through, with straight alpha.",
    ),
  ];
}
export function accountRows(): Row[] {
  const row = (
    id: string,
    group: string,
    label: string,
    control: Control,
    detail?: string,
  ): Row => ({
    id: `account.${id}`,
    section: "accounts",
    group,
    label,
    control,
    detail,
    disabled: true,
  });
  return [
    {
      ...row(
        "sign-in",
        "Powermove Account",
        "Powermove Account",
        "action",
        "Remember installs and publish to the Store.",
      ),
      action: "account.sign-in",
      actionLabel: "Sign In…",
    },
    row(
      "remember",
      "Powermove Account",
      "Remember Installs on This Account",
      "switch",
    ),
    {
      ...row(
        "bring-sign-ins",
        "Powermove Account",
        "Sign-Ins from the Original App",
        "action",
        "Copies your extension secrets and sign-ins, with one macOS Keychain request.",
      ),
      action: "account.bring-sign-ins",
      actionLabel: "Bring Sign-Ins from the Original App…",
      disabled: false,
    },
    // No status until the subscription service publishes one: a default
    // "Restart needed" would claim a state nobody observed (design review).
    ...["ChatGPT", "Claude"].map((name) =>
      row(name.toLowerCase(), "Subscriptions", name, "status"),
    ),
    {
      ...row("provider", "API or Local Model", "Connection Type", "choice"),
      options: choices([
        "OpenAI API",
        "Ollama",
        "LM Studio",
        "Other compatible provider",
      ]),
    },
    row(
      "url",
      "API or Local Model",
      "API Base URL",
      "text",
      "Base URL of the API, ending in /v1.",
    ),
    row(
      "key",
      "API or Local Model",
      "API Key",
      "secure",
      "Key saved securely. Leave blank to keep it.",
    ),
    row(
      "model",
      "API or Local Model",
      "Model Name",
      "text",
      "The exact model name your provider expects.",
    ),
    row(
      "vision",
      "API or Local Model",
      "This Model Supports Images",
      "switch",
      "The model can read pictures you attach to a message.",
    ),
    {
      ...row(
        "status",
        "API or Local Model",
        "Status",
        "action",
        "Restart Powermove to enable API and local connections.",
      ),
      action: "provider.test",
      actionLabel: "Test and Connect",
    },
  ];
}
export const extensionTypes = [
  "All Types",
  "Effects",
  "Layer Types",
  "Editor Tools",
  "Themes",
  "Commands",
] as const;
export type ExtensionType = (typeof extensionTypes)[number];
