/** The label a field inside a row is read by: one context for `powermove/ui`
 * rows and kit rows alike, so either kind of field takes either kind of row's
 * label. */
import { createContext } from "preact";
export const Label = createContext<string | undefined>(undefined);
