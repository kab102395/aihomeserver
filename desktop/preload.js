const { contextBridge, ipcRenderer } = require('electron');

contextBridge.exposeInMainWorld('aihomeserverLauncher', {
  openWorkerFolder: (kind) => ipcRenderer.invoke('open-worker-folder', kind),
  openLauncherLogFolder: () => ipcRenderer.invoke('open-launcher-log-folder'),
  getVmState: () => ipcRenderer.invoke('get-vm-state'),
  stopVm: () => ipcRenderer.invoke('stop-vm'),
  startVm: () => ipcRenderer.invoke('start-vm'),
});
