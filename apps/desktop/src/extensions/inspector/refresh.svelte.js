/* Reactive bridge for imperative engines that invoke PM.Inspector.refresh(). */
class InspectorRefresh {
  version = $state(0);

  bump() {
    this.version++;
  }
}

export const inspectorRefresh = new InspectorRefresh();
