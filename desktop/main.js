const { app, BrowserWindow } = require('electron');

const DEFAULT_URL = process.env.AIHOMESERVER_URL || 'http://127.0.0.1:3000';

function createWindow() {
  const win = new BrowserWindow({
    width: 1600,
    height: 1000,
    backgroundColor: '#111111',
    title: 'aihomeserver',
    autoHideMenuBar: true,
    webPreferences: {
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      preload: require('path').join(__dirname, 'preload.js'),
    },
  });

  win.loadURL(DEFAULT_URL);
}

app.whenReady().then(() => {
  createWindow();

  app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      createWindow();
    }
  });
});

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') {
    app.quit();
  }
});
