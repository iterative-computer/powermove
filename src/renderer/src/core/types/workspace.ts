import type { LayerType } from './project';
import type { Transform } from './transform';

export type WorkspaceDensity = 'compact' | 'normal' | 'comfy';
export type TimelineToolbarDensity = 'compact' | 'normal';
export type TimelineSurfaceOrder = 'normal' | 'reversed';
export type PreviewCornerRadius = 'square' | 'rounded';

export interface TimelineChrome {
  rowHeight: number;
  gutterWidth: number;
  rulerHeight: number;
  clipRadius: number;
  keyframeSize: number;
  showLayerNumbers: boolean;
  showTypeBadges: boolean;
  toolbarDensity: TimelineToolbarDensity;
}

export interface PanelSpec {
  id: string;
  flex?: true;
  collapsed?: true;
  size?: number;
  min?: number;
  title?: string;
}

export interface Dock {
  id: string;
  panels: PanelSpec[];
  size?: number;
  hidden?: true;
  flex?: true;
}

export type GeneratedStateValue = null | string | number | boolean;
export type JsonValue = GeneratedStateValue | JsonValue[] | { [key: string]: JsonValue };
export type GeneratedControlType =
  | 'slider'
  | 'text'
  | 'color'
  | 'fill'
  | 'toggle'
  | 'select'
  | 'button'
  | 'readout'
  | 'curve';
export type GeneratedReadoutSource =
  | 'selection.summary'
  | 'selection.count'
  | 'keyframes.summary'
  | 'keyframes.count';
export type GeneratedSelectValue = JsonValue;
export type GeneratedSelectOption = string | { v: GeneratedSelectValue; label: string };
export type EasingCurve = [number, number, number, number];

export interface TransformControlAction {
  type: 'transform';
  mode: 'preview' | 'apply';
  transform: Transform;
}

export interface HistoryControlAction {
  type: 'history';
  command: 'undo';
}

export interface ResetControlAction {
  type: 'reset';
}

export interface EasingControlAction {
  type: 'easing';
  mode: 'preview' | 'apply';
  scope: 'selected-keyframes';
  curveState: string;
  defaultCurve: EasingCurve;
}

export interface ScriptControlAction {
  type: 'script';
  mode: 'preview' | 'apply';
  code: string;
  label: string;
  requiredTypes: LayerType[];
}

export type GeneratedControlAction =
  | TransformControlAction
  | HistoryControlAction
  | ResetControlAction
  | EasingControlAction
  | ScriptControlAction;

interface GeneratedControlBase {
  type: GeneratedControlType;
  label: string;
  /** normalizeControl deliberately preserves authored extension metadata. */
  [key: string]: unknown;
}

export interface GeneratedReadoutControl extends GeneratedControlBase {
  type: 'readout';
  source: GeneratedReadoutSource;
}

export interface GeneratedButtonControl extends GeneratedControlBase {
  type: 'button';
  primary: boolean;
  action: GeneratedControlAction | null;
  cmd: string;
}

export interface GeneratedCurveControl extends GeneratedControlBase {
  type: 'curve';
  stateKey: string;
  def: EasingCurve;
  minY: number;
  maxY: number;
  presets: string[];
}

interface GeneratedBoundControlBase extends GeneratedControlBase {
  stateKey: string;
  param: string;
  target?: string;
  path?: string;
}

export interface GeneratedTextControl extends GeneratedBoundControlBase {
  type: 'text';
  def: string;
}

export interface GeneratedColorControl extends GeneratedBoundControlBase {
  type: 'color';
  def: string;
}

export interface GeneratedFillControl extends GeneratedBoundControlBase {
  type: 'fill';
  def: { [key: string]: JsonValue } | null;
}

export interface GeneratedToggleControl extends GeneratedBoundControlBase {
  type: 'toggle';
  def: boolean;
}

export interface GeneratedSelectControl extends GeneratedBoundControlBase {
  type: 'select';
  options: GeneratedSelectOption[];
  def: JsonValue;
}

export interface GeneratedSliderControl extends GeneratedBoundControlBase {
  type: 'slider';
  min: number;
  max: number;
  def: number;
  step: number;
  unit?: string;
}

export type GeneratedControl =
  | GeneratedReadoutControl
  | GeneratedButtonControl
  | GeneratedCurveControl
  | GeneratedTextControl
  | GeneratedColorControl
  | GeneratedFillControl
  | GeneratedToggleControl
  | GeneratedSelectControl
  | GeneratedSliderControl;

export interface GeneratedSection {
  id: string;
  title: string;
  size: number;
  state: Record<string, GeneratedStateValue>;
  controls: GeneratedControl[];
  name?: string;
  note?: string;
  tool?: string;
  /** normalizeWorkspace preserves authored section metadata. */
  [key: string]: unknown;
}

export interface WorkspaceTheme {
  accent?: string;
  radius?: number;
  [key: string]: unknown;
}

export interface WorkspaceFeatures {
  snapping?: boolean;
  autosave?: boolean;
  adaptiveQuality?: boolean;
  graphOnOpen?: boolean;
  [key: string]: unknown;
}

export interface WorkspaceChrome {
  previewCornerRadius: PreviewCornerRadius;
  timelineSurfaceOrder: TimelineSurfaceOrder;
  timelineSurfaceSchema: 2;
  timelineChromeSchema: 2;
  timeline: TimelineChrome;
  [key: string]: unknown;
}

export interface WorkspaceLayout {
  docks: Dock[];
  [key: string]: unknown;
}

export interface HiddenPanel {
  id: string;
  dockId: string;
  index: number;
  dockIndex: number;
  spec: PanelSpec & Record<string, unknown>;
  dock: (Partial<Pick<Dock, 'id' | 'size' | 'flex'>> & Record<string, unknown>) | null;
}

interface WorkspaceManifestBase {
  schemaVersion: 1;
  id: string;
  name: string;
  density: WorkspaceDensity;
  theme: WorkspaceTheme;
  chrome: WorkspaceChrome;
  features: WorkspaceFeatures;
  custom: GeneratedSection[];
  hiddenPanels: HiddenPanel[];
  layout: WorkspaceLayout;
  builtin?: boolean;
  base?: string;
  deletedAt?: number;
  /** normalizeWorkspace starts from a copy and retains unrecognized metadata. */
  [key: string]: unknown;
}

export type WorkspaceManifest = WorkspaceManifestBase & (
  | { scope: 'global'; projectId: null }
  | { scope: 'project'; projectId: string }
);
