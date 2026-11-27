/** Called only in the disposable synthetic reference fixture. Alias its
 * generated workspace identity just like the recorder aliases project ids;
 * never change the workspace or inspect authored strings/getters.
 */
export function admitFixtureWorkspace(root = globalThis) {
  const workspace = root.PM?.WS?.current;
  if (!workspace || typeof workspace !== "object") return;
  const id = Object.getOwnPropertyDescriptor(workspace, "id");
  if (id && "value" in id && typeof id.value === "string")
    root.__PMTrace.recorder.token(id.value, "workspace");
}
