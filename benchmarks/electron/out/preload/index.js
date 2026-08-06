let electron = require("electron");
//#region src/preload/index.ts
electron.contextBridge.exposeInMainWorld("api", {
	ipc: (channel) => electron.ipcRenderer.invoke(channel),
	logResults: (lines) => electron.ipcRenderer.invoke("log-results", lines)
});
//#endregion
