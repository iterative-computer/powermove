// Browser bundle for reference renders: the production SceneRuntime and glTF import,
// unchanged, so the corpus is rendered exactly as Powermove renders 3D layers today.
// The recorder hooks WebGL2 before three.js creates its context.
import { installRecorder } from './recorder';
installRecorder();
import './page';
