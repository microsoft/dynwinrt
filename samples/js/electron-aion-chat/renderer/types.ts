// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

export interface RuntimeInfo {
  architecture: string;
  runtimeDependency: string;
  loadMilliseconds: number;
}

export interface GenerationMetrics {
  chunks: number;
  firstTokenMilliseconds: number | null;
  totalMilliseconds: number;
}

export type GenerationResult = {
  text: string;
  streamedText: string;
  metrics: GenerationMetrics;
} & ({ canceled: false; status: number } | { canceled: true; status: null });

export interface AionApi {
  initialize(): Promise<RuntimeInfo>;
  generate(
    prompt: string,
    onProgress?: (chunk: string) => void,
  ): Promise<GenerationResult>;
  cancel(): boolean;
  resetConversation(): Promise<boolean>;
}

export interface ChatMessage {
  id: number;
  role: "user" | "assistant";
  text: string;
  state: "complete" | "streaming" | "canceled" | "error";
}

export interface ModelStatus {
  state: "loading" | "ready" | "working" | "error";
  text: string;
}

declare global {
  interface Window {
    aionSample: AionApi;
  }
}
