// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import { useLayoutEffect, useRef } from "react";
import type { ChatMessage } from "../types";

export function MessageList({ messages }: { messages: ChatMessage[] }) {
  const container = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    if (container.current) {
      container.current.scrollTop = container.current.scrollHeight;
    }
  }, [messages]);

  return (
    <div id="messages" className="messages" aria-live="polite" ref={container}>
      {messages.map((message) => (
        <article className={`message ${message.role}`} key={message.id}>
          <div className="avatar" aria-hidden="true">
            {message.role === "assistant" ? "A" : "You"}
          </div>
          <div className="message-body">
            <div className="message-author">
              {message.role === "assistant" ? "Aion" : "You"}
            </div>
            <div className={`message-text ${message.state}`}>
              {message.state === "streaming" && !message.text
                ? "Thinking\u2026"
                : message.text}
            </div>
          </div>
        </article>
      ))}
    </div>
  );
}
