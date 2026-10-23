/* Reactive bridge for the inspector service's imperative refresh hook. */
class InspectorRefresh {
  version = $state(0);

  bump() {
    this.version++;
  }
}

export const inspectorRefresh = new InspectorRefresh();
