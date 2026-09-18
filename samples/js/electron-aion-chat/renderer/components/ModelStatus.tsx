// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

import type { ModelStatus as Status } from "../types";

export function ModelStatus({ status }: { status: Status }) {
  return (
    <div
      id="model-status"
      className="status-pill"
      data-state={status.state}
      role="status"
    >
      <span className="status-dot" aria-hidden="true" />
      <span id="model-status-text">{status.text}</span>
    </div>
  );
}
