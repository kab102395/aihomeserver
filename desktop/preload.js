const { contextBridge, ipcRenderer } = require('electron');

contextBridge.exposeInMainWorld('aihomeserverLauncher', {
  openWorkerFolder: (kind) => ipcRenderer.invoke('open-worker-folder', kind),
});
