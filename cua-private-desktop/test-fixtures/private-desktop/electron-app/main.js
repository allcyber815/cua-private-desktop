const { app, BrowserWindow } = require('electron');
const path = require('path');

const prefix = '--webgpt-user-data-dir=';
const custom = process.argv.find(a => a.startsWith(prefix));
if (custom) app.setPath('userData', custom.slice(prefix.length));
app.commandLine.appendSwitch('force-renderer-accessibility');

function createWindow() {
  const win = new BrowserWindow({
    width: 945,
    height: 1012,
    show: true,
    title: 'WebGPT Background Fixture Electron',
    webPreferences: { contextIsolation: true, sandbox: true }
  });
  win.loadFile(path.join(__dirname, 'index.html'));
}

app.whenReady().then(createWindow);
app.on('window-all-closed', () => app.quit());
