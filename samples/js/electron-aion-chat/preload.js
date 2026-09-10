// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

const { contextBridge } = require("electron");
const { AionService } = require("./preload/aion-service.js");

const service = new AionService();
window.addEventListener("unload", () => service.dispose(), { once: true });
contextBridge.exposeInMainWorld(
  "aionSample",
  Object.freeze({
    initialize: () => service.initialize(),
    generate: (prompt, onProgress) => service.generate(prompt, onProgress),
    cancel: () => service.cancel(),
    resetConversation: () => service.resetConversation(),
  }),
);
