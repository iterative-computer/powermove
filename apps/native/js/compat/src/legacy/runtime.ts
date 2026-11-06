/** The reference's own pure engines, installed in its dependency order. They
 * read and write `PM.proj` exactly as they do in Electron; the realm's write
 * barrier and native capabilities below them are the only difference.
 */
import type { Data } from "../types";
import { install as installUtil } from "../frozen/reference/legacy/core/pure-util";
import { install as installUiState } from "../frozen/reference/legacy/core/ui-state";
import { install as installEasing } from "../frozen/reference/legacy/core/easing";
import { install as installSelection } from "../frozen/reference/legacy/core/selection";
import { install as installAnim } from "../frozen/reference/legacy/core/anim";
import { installLayerGroups } from "../frozen/reference/legacy/core/layer-groups";
import { installProjectIndex } from "../frozen/reference/legacy/core/project-index";
import {
  installMediaTiming,
  selectLayers,
} from "../frozen/reference/legacy/core/pure-model";

export function installLegacyRuntime(PM: Data): void {
  const nativeTouch = PM.touch;
  installUtil(PM);
  installUiState(PM);
  installEasing(PM);
  installProjectIndex(PM);
  PM.selectLayers = (ids: unknown, add = false) => selectLayers(PM, ids, add);
  PM.selLayers = () => PM.ProjectIndex.selectedLayers(PM.sel.layers);
  PM.firstSel = () => PM.selLayers()[0] || null;
  installSelection(PM);
  installAnim(PM);
  installLayerGroups(PM);
  installMediaTiming(PM);
  const touch = PM.touch;
  PM.touch = () => {
    touch();
    nativeTouch();
  };
}
