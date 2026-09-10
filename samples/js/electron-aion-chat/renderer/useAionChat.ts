// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { useCallback, useEffect, useRef, useState } from "react";
import type { AionApi, ChatMessage, ModelStatus, RuntimeInfo } from "./types";

const READY_STATUS: ModelStatus = { state: "ready", text: "Ready on NPU" };
const DEMO_PROMPT =
  "Explain why on-device AI is useful in two concise sentences. Do not use Markdown.";
const CONTEXT_FULL_MESSAGE =
  "The conversation no longer fits in the model context. Start a new conversation to continue.";

type Activity = "loading" | "idle" | "generating" | "stopping" | "resetting";
type PendingAction = { kind: "generation" | "reset"; stopping?: boolean };

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function useAionChat(api: AionApi, demo: boolean) {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [runtime, setRuntime] = useState<RuntimeInfo | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [activity, setActivity] = useState<Activity>("loading");
  const [contextFull, setContextFull] = useState(false);
  const [status, setStatus] = useState<ModelStatus>({
    state: "loading",
    text: "Loading model",
  });
  const [metrics, setMetrics] = useState("Waiting for the model");
  const mounted = useRef(false);
  const nextId = useRef(0);
  const activeAction = useRef<PendingAction | null>(null);
  const demoStarted = useRef(false);

  useEffect(() => {
    let current = true;
    mounted.current = true;

    async function initialize() {
      try {
        const info = await api.initialize();
        if (!current) return;
        setRuntime(info);
        setMessages([
          {
            id: nextId.current++,
            role: "assistant",
            text: "The Aion model is ready on this device. Ask a question or choose a suggested prompt.",
            state: "complete",
          },
        ]);
        setStatus(READY_STATUS);
        setMetrics("Model ready \u00b7 conversation context created");
      } catch (error) {
        if (!current) return;
        const message = errorMessage(error);
        setLoadError(message);
        setStatus({ state: "error", text: "Unavailable" });
        setMessages([
          {
            id: nextId.current++,
            role: "assistant",
            text: `Unable to use Aion: ${message}`,
            state: "error",
          },
        ]);
        setMetrics("Model initialization failed");
      } finally {
        if (current) setActivity("idle");
      }
    }

    void initialize();
    return () => {
      current = false;
      mounted.current = false;
      const action = activeAction.current;
      activeAction.current = null;
      if (action?.kind === "generation") {
        try {
          api.cancel();
        } catch (error) {
          console.error(
            "Failed to cancel generation on renderer unmount.",
            error,
          );
        }
      }
      // The preload owns native disposal; React effect cleanup must not close it.
    };
  }, [api]);

  const canSend = runtime !== null && activity === "idle" && !contextFull;
  const canReset = runtime !== null && activity === "idle";

  const send = useCallback(
    async (prompt: string) => {
      const text = prompt.trim();
      if (!canSend || !text || activeAction.current) return;

      const action: PendingAction = { kind: "generation" };
      activeAction.current = action;
      const userId = nextId.current++;
      const responseId = nextId.current++;
      const isCurrent = () =>
        mounted.current && activeAction.current === action;
      const finish = (text: string, state: ChatMessage["state"]) => {
        setMessages((previous) =>
          previous.map((message) =>
            message.id === responseId ? { ...message, text, state } : message,
          ),
        );
      };

      setMessages((previous) => [
        ...previous,
        { id: userId, role: "user", text, state: "complete" },
        { id: responseId, role: "assistant", text: "", state: "streaming" },
      ]);
      setActivity("generating");
      setStatus({ state: "working", text: "Generating" });
      setMetrics("Waiting for the first token\u2026");

      try {
        const result = await api.generate(text, (chunk) => {
          if (!isCurrent() || action.stopping) return;
          setMessages((previous) =>
            previous.map((message) =>
              message.id === responseId && message.state === "streaming"
                ? { ...message, text: message.text + chunk }
                : message,
            ),
          );
        });
        if (!isCurrent()) return;

        if (result.canceled) {
          finish(result.text || "Generation stopped.", "canceled");
          setStatus(READY_STATUS);
          setMetrics(
            `Stopped after ${result.metrics.totalMilliseconds} ms \u00b7 ${result.metrics.chunks} chunks`,
          );
        } else if (result.status === 0) {
          finish(result.text, "complete");
          setStatus(READY_STATUS);
          setMetrics(
            `Complete \u00b7 first token ${result.metrics.firstTokenMilliseconds ?? "n/a"} ms \u00b7 ` +
              `${result.metrics.totalMilliseconds} ms total \u00b7 ${result.metrics.chunks} chunks`,
          );
        } else if (result.status === 3) {
          finish(
            [result.text, CONTEXT_FULL_MESSAGE].filter(Boolean).join("\n\n"),
            "error",
          );
          setContextFull(true);
          setStatus({ state: "error", text: "Context full" });
          setMetrics("Context limit reached");
        } else {
          finish(
            result.text ||
              `Aion returned an error response (status ${result.status}).`,
            "error",
          );
          setStatus({ state: "error", text: "Model error" });
          setMetrics(`Aion response status ${result.status}`);
        }
      } catch (error) {
        if (!isCurrent()) return;
        finish(`Error: ${errorMessage(error)}`, "error");
        setStatus({ state: "error", text: "Generation failed" });
        setMetrics("Generation failed");
      } finally {
        if (activeAction.current === action) {
          activeAction.current = null;
          if (mounted.current) setActivity("idle");
        }
      }
    },
    [api, canSend],
  );

  function cancel() {
    const action = activeAction.current;
    if (action?.kind !== "generation" || action.stopping) return;
    try {
      if (api.cancel()) {
        action.stopping = true;
        setActivity("stopping");
        setStatus({ state: "working", text: "Stopping" });
      }
    } catch (error) {
      setStatus({ state: "error", text: "Unable to stop generation" });
      setMetrics(errorMessage(error));
    }
  }

  async function resetConversation() {
    if (!canReset || activeAction.current) return;
    const action: PendingAction = { kind: "reset" };
    activeAction.current = action;
    setActivity("resetting");
    setStatus({ state: "working", text: "Starting a new conversation" });
    try {
      await api.resetConversation();
      if (!mounted.current || activeAction.current !== action) return;
      setMessages([
        {
          id: nextId.current++,
          role: "assistant",
          text: "New local conversation started. What would you like to explore?",
          state: "complete",
        },
      ]);
      setContextFull(false);
      setStatus(READY_STATUS);
      setMetrics("Conversation context reset");
    } catch (error) {
      if (!mounted.current || activeAction.current !== action) return;
      const message: ChatMessage = {
        id: nextId.current++,
        role: "assistant",
        text: `Unable to reset the conversation: ${errorMessage(error)}`,
        state: "error",
      };
      setMessages((previous) => [...previous, message]);
      setStatus({ state: "error", text: "Conversation reset failed" });
      setMetrics("Conversation reset failed");
    } finally {
      if (activeAction.current === action) {
        activeAction.current = null;
        if (mounted.current) setActivity("idle");
      }
    }
  }

  useEffect(() => {
    if (!demo || !canSend || demoStarted.current) return;
    demoStarted.current = true;
    void send(DEMO_PROMPT);
  }, [demo, canSend, send]);

  return {
    messages,
    runtime,
    loadError,
    activity,
    status,
    metrics,
    canSend,
    canReset,
    send,
    cancel,
    resetConversation,
  };
}
