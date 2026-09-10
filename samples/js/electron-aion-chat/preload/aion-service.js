// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

const { performance } = require("node:perf_hooks");
const { hasPackageIdentity } = require("@microsoft/dynwinrt");
const {
  LanguageModel,
} = require("../generated/aion-instruct-preview/text/LanguageModel.js");

function closeProjected(value) {
  try {
    value?.close();
  } catch (error) {
    console.error("Failed to close an Aion WinRT value.", error);
  }
}

class AionService {
  constructor() {
    this.model = undefined;
    this.context = undefined;
    this.initialization = undefined;
    this.loadMilliseconds = undefined;
    this.activeController = undefined;
    this.activeOperation = undefined;
    this.disposed = false;
  }

  ensureRuntime() {
    if (process.platform !== "win32" || process.arch !== "arm64") {
      throw new Error(
        `Aion Instruct Preview 1.0 requires ARM64 Windows; this Electron process is ${process.platform}/${process.arch}.`,
      );
    }
    if (!hasPackageIdentity()) {
      throw new Error(
        "Aion requires the manifest package graph. Launch this sample with npm start so WinApp CLI can apply the Electron debug identity.",
      );
    }
  }

  async initialize() {
    if (this.disposed) throw new Error("The Aion sample is shutting down.");
    if (this.model && this.context) return this.runtimeInfo();
    if (!this.initialization) {
      this.initialization = (async () => {
        this.ensureRuntime();
        const started = performance.now();
        const createdModel = await LanguageModel.createAsync();
        if (!createdModel) {
          throw new Error("Aion returned a null LanguageModel.");
        }
        let createdContext;
        try {
          createdContext = createdModel.createContext();
        } catch (error) {
          closeProjected(createdModel);
          throw error;
        }
        if (!createdContext) {
          closeProjected(createdModel);
          throw new Error("Aion returned a null LanguageModelContext.");
        }
        if (this.disposed) {
          closeProjected(createdContext);
          closeProjected(createdModel);
          throw new Error(
            "The Aion sample was closed while loading the model.",
          );
        }
        this.model = createdModel;
        this.context = createdContext;
        this.loadMilliseconds = Math.round(performance.now() - started);
        return this.runtimeInfo();
      })().catch((error) => {
        this.initialization = undefined;
        throw error;
      });
    }
    return this.initialization;
  }

  runtimeInfo() {
    return {
      architecture: process.arch,
      runtimeDependency: "Aion Instruct Preview >= 1.0.0.0 (manifest)",
      loadMilliseconds: this.loadMilliseconds,
    };
  }

  async generate(prompt, onProgress) {
    const text = String(prompt ?? "").trim();
    if (!text) throw new Error("Enter a prompt before generating.");
    if (text.length > 4000) {
      throw new Error(
        "Prompts are limited to 4,000 characters in this sample.",
      );
    }
    if (this.activeOperation) {
      throw new Error("Another generation is already in progress.");
    }

    const controller = new AbortController();
    this.activeController = controller;
    this.activeOperation = "generation";
    const started = performance.now();
    let firstTokenAt;
    let streamedText = "";
    let chunks = 0;

    try {
      await this.initialize();
      const operation = this.model.generateResponseAsync(
        this.context,
        text,
        controller.signal,
      );
      operation.progress((delta) => {
        if (controller.signal.aborted) return;
        const chunk = String(delta);
        if (firstTokenAt === undefined) firstTokenAt = performance.now();
        streamedText += chunk;
        chunks += 1;
        try {
          onProgress?.(chunk);
        } catch (error) {
          console.error("The renderer progress callback failed.", error);
        }
      });

      const result = await operation;
      return {
        canceled: false,
        status: result.status,
        text: result.text,
        streamedText,
        metrics: {
          chunks,
          firstTokenMilliseconds:
            firstTokenAt === undefined
              ? null
              : Math.round(firstTokenAt - started),
          totalMilliseconds: Math.round(performance.now() - started),
        },
      };
    } catch (error) {
      if (!controller.signal.aborted) throw error;
      return {
        canceled: true,
        status: null,
        text: streamedText,
        streamedText,
        metrics: {
          chunks,
          firstTokenMilliseconds:
            firstTokenAt === undefined
              ? null
              : Math.round(firstTokenAt - started),
          totalMilliseconds: Math.round(performance.now() - started),
        },
      };
    } finally {
      if (this.activeController === controller) {
        this.activeController = undefined;
      }
      if (this.activeOperation === "generation") {
        this.activeOperation = undefined;
      }
    }
  }

  cancel() {
    if (!this.activeController) return false;
    this.activeController.abort(new Error("Generation stopped by the user."));
    return true;
  }

  async resetConversation() {
    if (this.activeOperation) {
      throw new Error(
        "Stop the current generation before starting a new conversation.",
      );
    }
    this.activeOperation = "reset";
    try {
      await this.initialize();
      const previous = this.context;
      this.context = this.model.createContext();
      if (!this.context) {
        this.context = previous;
        throw new Error("Aion returned a null replacement context.");
      }
      closeProjected(previous);
      return true;
    } finally {
      if (this.activeOperation === "reset") {
        this.activeOperation = undefined;
      }
    }
  }

  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    this.cancel();
    closeProjected(this.context);
    closeProjected(this.model);
    this.context = undefined;
    this.model = undefined;
  }
}

module.exports = { AionService };
