/** Electron entry; the same runtime is embedded by the plain Node host. */
import { startEngineProcess } from './engine-runtime';
startEngineProcess(process.parentPort);
