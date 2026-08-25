'use strict';

const { contextBridge, ipcRenderer } = require('electron');

contextBridge.exposeInMainWorld('probe', Object.freeze({
  mode: ipcRenderer.sendSync('probe:mode'),
  storeSnapshot: () => ipcRenderer.invoke('probe:store-snapshot'),
  fixtures: () => ipcRenderer.invoke('probe:fixtures'),
  clearConsoleMessages: () => ipcRenderer.invoke('probe:console-clear'),
  consoleMessages: () => ipcRenderer.invoke('probe:console-messages'),
  mainStatic: () => ipcRenderer.invoke('probe:main-static'),
  mainMemory: () => ipcRenderer.invoke('probe:memory'),
  permissionLog: () => ipcRenderer.invoke('probe:permission-log'),
  syntheticClick: () => ipcRenderer.invoke('probe:synthetic-click'),
  focusWindow: () => ipcRenderer.invoke('probe:focus-window'),
  capturePage: () => ipcRenderer.invoke('probe:capture-page'),
  runStrict: () => ipcRenderer.invoke('probe:run-strict'),
  strictComplete: (result) => ipcRenderer.invoke('probe:strict-submit', result),
  big: (payload, sentAtEpochMs) => ipcRenderer.invoke('probe:big', payload, sentAtEpochMs),
  bigReverse: (byteLength) => ipcRenderer.invoke('probe:big-reverse', byteLength),
  checkpoint: (key, result) => ipcRenderer.invoke('probe:checkpoint', key, result),
  complete: (results) => ipcRenderer.send('probe:complete', results),
}));
