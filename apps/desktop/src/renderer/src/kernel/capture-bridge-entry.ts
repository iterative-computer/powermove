/* Side-effect entry, imported first by main.ts before any extension can run. */
import { captureBridge } from './capture-bridge';

captureBridge();
