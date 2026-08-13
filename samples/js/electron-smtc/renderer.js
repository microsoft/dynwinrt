// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

const status = document.querySelector("#status");
const events = document.querySelector("#events");
const eventLines = [];

function options() {
  return {
    title: document.querySelector("#title").value,
    artist: document.querySelector("#artist").value,
    durationSeconds: Number(document.querySelector("#duration").value),
    positionSeconds: Number(document.querySelector("#position").value),
  };
}

function show(value) {
  status.textContent = JSON.stringify(value, null, 2);
}

window.showValidationResult = show;

async function run(action) {
  try {
    show(await action());
  } catch (error) {
    show({ error: error instanceof Error ? error.message : String(error) });
  }
}

window.systemMediaControls.onEvent((event) => {
  eventLines.unshift(JSON.stringify(event));
  events.textContent = eventLines.slice(0, 30).join("\n");
});

document
  .querySelector("#initialize")
  .addEventListener("click", () =>
    run(() => window.systemMediaControls.initialize(options())),
  );
document
  .querySelector("#update")
  .addEventListener("click", () =>
    run(() => window.systemMediaControls.update(options())),
  );
document.querySelector("#play").addEventListener("click", () =>
  run(async () => ({
    accepted: await window.systemMediaControls.control("play"),
    snapshot: await window.systemMediaControls.snapshot(),
  })),
);
document.querySelector("#pause").addEventListener("click", () =>
  run(async () => ({
    accepted: await window.systemMediaControls.control("pause"),
    snapshot: await window.systemMediaControls.snapshot(),
  })),
);
document.querySelector("#seek").addEventListener("click", () =>
  run(async () => ({
    accepted: await window.systemMediaControls.control(
      "seek",
      Number(document.querySelector("#position").value),
    ),
    snapshot: await window.systemMediaControls.snapshot(),
  })),
);
document
  .querySelector("#validate")
  .addEventListener("click", () =>
    run(() => window.systemMediaControls.validate()),
  );
