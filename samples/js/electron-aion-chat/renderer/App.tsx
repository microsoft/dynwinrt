// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { Composer } from "./components/Composer";
import { MessageList } from "./components/MessageList";
import { ModelStatus } from "./components/ModelStatus";
import { useAionChat } from "./useAionChat";

const suggestions = [
  {
    label: "Why on-device AI?",
    prompt: "Explain why on-device AI is useful in two concise sentences.",
  },
  {
    label: "Windows on ARM poem",
    prompt: "Write a four-line poem about Windows on ARM.",
  },
  {
    label: "Offline app ideas",
    prompt: "Give me three practical ideas for an offline Electron app.",
  },
];

export function App() {
  const demo = new URLSearchParams(window.location.search).get("demo") === "1";
  const chat = useAionChat(window.aionSample, demo);
  const runtimeDetail = chat.loadError
    ? chat.loadError
    : chat.runtime
      ? `${chat.runtime.architecture.toUpperCase()} \u00b7 ${chat.runtime.runtimeDependency} \u00b7 loaded in ${chat.runtime.loadMilliseconds} ms`
      : "Preparing the local model\u2026";

  return (
    <>
      <header className="titlebar">
        <div className="brand-mark" aria-hidden="true">
          A
        </div>
        <div className="brand-title">
          <span className="title">Aion Chat</span>
          <span className="preview-badge">Preview</span>
        </div>
        <ModelStatus status={chat.status} />
      </header>

      <main>
        <section className="hero">
          <div>
            <h1>Chat with Aion</h1>
            <p className="hero-copy">
              Ask a question, draft something, or explore an idea. All on your
              device.
            </p>
          </div>
          <div className="runtime-card">
            <div className="runtime-label">Local runtime</div>
            <strong id="runtime-name">Aion Instruct Preview 1.0</strong>
            <span id="runtime-detail" title={runtimeDetail}>
              {runtimeDetail}
            </span>
          </div>
        </section>

        <section className="chat-shell" aria-label="Aion chat">
          <div className="chat-toolbar">
            <div>
              <strong>Conversation</strong>
              <span>Runs locally; prompts do not leave this device.</span>
            </div>
            <button
              id="new-chat"
              className="secondary-button"
              type="button"
              disabled={!chat.canReset}
              onClick={() => void chat.resetConversation()}
            >
              <span className="button-icon" aria-hidden="true">
                +
              </span>
              {chat.activity === "resetting"
                ? "Resetting\u2026"
                : "New conversation"}
            </button>
          </div>

          <MessageList messages={chat.messages} />

          <div className="suggestions" aria-label="Suggested prompts">
            {suggestions.map((suggestion) => (
              <button
                key={suggestion.label}
                type="button"
                data-prompt={suggestion.prompt}
                disabled={!chat.canSend}
                onClick={() => void chat.send(suggestion.prompt)}
              >
                {suggestion.label}
              </button>
            ))}
          </div>

          <Composer
            disabled={!chat.canSend}
            generating={
              chat.activity === "generating" || chat.activity === "stopping"
            }
            stopping={chat.activity === "stopping"}
            onSend={chat.send}
            onCancel={chat.cancel}
          />
        </section>

        <footer>
          <span id="metrics">{chat.metrics}</span>
          <span>Aion can make mistakes. Review its responses.</span>
        </footer>
      </main>
    </>
  );
}
