const { contextBridge, ipcRenderer } = require('electron');

contextBridge.exposeInMainWorld('aihomeserverLauncher', {
  openWorkerFolder: (kind) => ipcRenderer.invoke('open-worker-folder', kind),
  openLauncherLogFolder: () => ipcRenderer.invoke('open-launcher-log-folder'),
  openHostRepoFolder: () => ipcRenderer.invoke('open-host-repo-folder'),
  getVmState: () => ipcRenderer.invoke('get-vm-state'),
  stopVm: () => ipcRenderer.invoke('stop-vm'),
  startVm: () => ipcRenderer.invoke('start-vm'),
  rebuildApp: () => ipcRenderer.invoke('rebuild-app'),
  getDesktopSettings: () => ipcRenderer.invoke('get-desktop-settings'),
  saveDesktopSettings: (settings) => ipcRenderer.invoke('save-desktop-settings', settings),
});
