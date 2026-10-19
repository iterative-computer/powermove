import { startEngineProcess } from '../../main/engine/engine-runtime';
import { childParent } from './ports';
startEngineProcess(childParent());
