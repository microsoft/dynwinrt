// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { FormEvent } from "react";

interface ComposerProps {
  disabled: boolean;
  generating: boolean;
  stopping: boolean;
  onSend: (prompt: string) => Promise<void>;
  onCancel: () => void;
}

export function Composer({
  disabled,
  generating,
  stopping,
  onSend,
  onCancel,
}: ComposerProps) {
  const [prompt, setPrompt] = useState("");
  const input = useRef<HTMLTextAreaElement>(null);

  useLayoutEffect(() => {
    if (input.current) {
      input.current.style.height = "auto";
      input.current.style.height = `${input.current.scrollHeight}px`;
    }
  }, [prompt]);

  useEffect(() => {
    if (!disabled) input.current?.focus();
  }, [disabled]);

  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (disabled || !prompt.trim()) return;
    void onSend(prompt);
    setPrompt("");
  }

  return (
    <form id="prompt-form" className="composer" onSubmit={submit}>
      <label className="sr-only" htmlFor="prompt">
        Message Aion
      </label>
      <textarea
        ref={input}
        id="prompt"
        rows={1}
        maxLength={4000}
        placeholder={"Message Aion\u2026"}
        disabled={disabled}
        value={prompt}
        onChange={(event) => setPrompt(event.target.value)}
        onKeyDown={(event) => {
          if (
            event.key === "Enter" &&
            !event.shiftKey &&
            !event.nativeEvent.isComposing
          ) {
            event.preventDefault();
            event.currentTarget.form?.requestSubmit();
          }
        }}
      />
      <div className="composer-actions">
        <span className="composer-model">
          Aion Instruct
          <span className="local-badge">Local</span>
        </span>
        <span id="character-count">{prompt.length} / 4000</span>
        {generating && (
          <button
            id="stop"
            className="stop-button"
            type="button"
            disabled={stopping}
            onClick={onCancel}
          >
            <span className="stop-icon" aria-hidden="true" />
            {stopping ? "Stopping\u2026" : "Stop"}
          </button>
        )}
        <button
          id="send"
          className="send-button"
          type="submit"
          aria-label="Send message"
          title="Send message (Enter)"
          disabled={disabled || !prompt.trim()}
        >
          <svg
            width="16"
            height="16"
            viewBox="0 0 16 16"
            fill="none"
            aria-hidden="true"
          >
            <path
              d="M8 13V3M3.5 7.5 8 3l4.5 4.5"
              stroke="currentColor"
              strokeWidth="1.75"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
        </button>
      </div>
    </form>
  );
}
