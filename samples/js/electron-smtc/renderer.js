// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

const api = window.systemMediaControls;
const element = (selector) => document.querySelector(selector);
const eventLines = [];
let initialized = false;
let snapshotInFlight = false;
let sessionsInFlight = false;
let refreshTimer;
let renderedTrackKey = "";

const SMTC_STATUS_NAMES = {
  0: "Closed",
  1: "Changing",
  2: "Stopped",
  3: "Playing",
  4: "Paused",
};

const GSMTC_STATUS_NAMES = {
  0: "Closed",
  1: "Opened",
  2: "Changing",
  3: "Stopped",
  4: "Playing",
  5: "Paused",
};

const REPEAT_MODE_NAMES = {
  0: "No repeat",
  1: "Repeat track",
  2: "Repeat list",
};

const SYNTH_PROGRAMS = [
  {
    bpm: 104,
    wave: "triangle",
    notes: [72, 76, 79, 84, 79, 76, 74, 79, 81, 77, 74, 69, 72, 76, 79, 76],
    bass: [48, 48, 45, 43],
  },
  {
    bpm: 118,
    wave: "sawtooth",
    notes: [67, 70, 74, 77, 74, 70, 65, 69, 72, 76, 72, 69, 67, 72, 74, 70],
    bass: [43, 46, 41, 45],
  },
  {
    bpm: 82,
    wave: "sine",
    notes: [69, 76, 72, 79, 74, 81, 76, 72, 67, 74, 71, 78, 72, 79, 74, 71],
    bass: [45, 41, 43, 40],
  },
];

function midiToFrequency(note) {
  return 440 * 2 ** ((note - 69) / 12);
}

class DemoSynth {
  constructor(available) {
    this.available = available;
    this.enabled = available;
    this.context = null;
    this.master = null;
    this.scheduler = null;
    this.latestPublisher = null;
    this.trackIndex = -1;
    this.playbackRate = 1;
    this.step = 0;
    this.nextNoteTime = 0;
    this.lastPosition = 0;
    this.lastSyncAt = performance.now();
    this.playing = false;
    this.#report(
      available
        ? "Ready when playback starts"
        : "Muted during automated validation",
    );
  }

  #report(message) {
    element("#audio-status").textContent = message;
    const toggle = element("#audio-toggle");
    toggle.disabled = !this.available;
    toggle.textContent = this.enabled ? "Mute" : "Enable";
  }

  #ensureContext() {
    if (!this.available) return null;
    if (!this.context) {
      this.context = new AudioContext({ latencyHint: "interactive" });
      this.master = this.context.createGain();
      this.master.gain.value = 0.0001;
      this.master.connect(this.context.destination);
    }
    return this.context;
  }

  async unlock() {
    if (!this.enabled) return;
    const context = this.#ensureContext();
    if (context?.state === "suspended") await context.resume();
  }

  #program() {
    return SYNTH_PROGRAMS[this.trackIndex % SYNTH_PROGRAMS.length];
  }

  #baseStepSeconds() {
    return 60 / this.#program().bpm / 2;
  }

  #resetPosition(publisher) {
    const context = this.#ensureContext();
    const program = this.#program();
    this.step =
      Math.floor(publisher.positionSeconds / this.#baseStepSeconds()) %
      program.notes.length;
    this.nextNoteTime = (context?.currentTime ?? 0) + 0.03;
  }

  #scheduleVoice(note, when, duration, level, wave) {
    const context = this.context;
    const oscillator = context.createOscillator();
    const gain = context.createGain();
    const attack = Math.min(0.025, duration * 0.15);

    oscillator.type = wave;
    oscillator.frequency.setValueAtTime(midiToFrequency(note), when);
    gain.gain.setValueAtTime(0.0001, when);
    gain.gain.exponentialRampToValueAtTime(level, when + attack);
    gain.gain.exponentialRampToValueAtTime(0.0001, when + duration);
    oscillator.connect(gain);
    gain.connect(this.master);
    oscillator.start(when);
    oscillator.stop(when + duration + 0.02);
    oscillator.addEventListener(
      "ended",
      () => {
        oscillator.disconnect();
        gain.disconnect();
      },
      { once: true },
    );
  }

  #scheduleAhead() {
    if (!this.playing || !this.context) return;
    const program = this.#program();
    const stepSeconds = this.#baseStepSeconds() / this.playbackRate;
    const horizon = this.context.currentTime + 0.14;

    while (this.nextNoteTime < horizon) {
      const noteIndex = this.step % program.notes.length;
      this.#scheduleVoice(
        program.notes[noteIndex],
        this.nextNoteTime,
        stepSeconds * 0.82,
        0.32,
        program.wave,
      );
      if (this.step % 2 === 0) {
        const bassIndex = Math.floor(this.step / 4) % program.bass.length;
        this.#scheduleVoice(
          program.bass[bassIndex],
          this.nextNoteTime,
          stepSeconds * 1.7,
          0.16,
          "sine",
        );
      }
      this.nextNoteTime += stepSeconds;
      this.step += 1;
    }
  }

  #stop(message) {
    if (this.scheduler) {
      clearInterval(this.scheduler);
      this.scheduler = null;
    }
    this.playing = false;
    if (this.master && this.context) {
      const now = this.context.currentTime;
      this.master.gain.cancelScheduledValues(now);
      this.master.gain.setTargetAtTime(0.0001, now, 0.025);
    }
    this.#report(message);
  }

  async sync(publisher) {
    this.latestPublisher = publisher;
    if (!this.available) return;
    if (!this.enabled) {
      this.#stop("Audio muted");
      return;
    }

    const shouldPlay = publisher.playbackStatus === 3;
    const trackChanged = publisher.trackIndex !== this.trackIndex;
    const elapsed =
      ((performance.now() - this.lastSyncAt) / 1000) * this.playbackRate;
    const predictedPosition = this.lastPosition + elapsed;
    const positionDrift = Math.abs(
      publisher.positionSeconds - predictedPosition,
    );

    this.trackIndex = publisher.trackIndex;
    this.playbackRate = Math.max(0.25, publisher.playbackRate);
    this.lastPosition = publisher.positionSeconds;
    this.lastSyncAt = performance.now();

    if (!shouldPlay) {
      this.#stop(
        `${SMTC_STATUS_NAMES[publisher.playbackStatus] ?? "Playback"} - audio idle`,
      );
      return;
    }

    await this.unlock();
    if (trackChanged || !this.playing || positionDrift > 1.5) {
      this.#resetPosition(publisher);
    }
    this.playing = true;
    const now = this.context.currentTime;
    this.master.gain.cancelScheduledValues(now);
    this.master.gain.setValueAtTime(
      Math.max(0.0001, this.master.gain.value),
      now,
    );
    this.master.gain.linearRampToValueAtTime(0.12, now + 0.05);
    if (!this.scheduler) {
      this.scheduler = setInterval(() => this.#scheduleAhead(), 30);
    }
    this.#scheduleAhead();
    this.#report(`Playing locally generated track ${publisher.trackIndex + 1}`);
  }

  async toggle() {
    this.enabled = !this.enabled;
    if (!this.enabled) {
      this.#stop("Audio muted");
      return;
    }
    await this.unlock();
    if (this.latestPublisher) await this.sync(this.latestPublisher);
    else this.#report("Ready when playback starts");
  }

  dispose() {
    this.#stop("Audio stopped");
    void this.context?.close();
    this.context = null;
    this.master = null;
  }
}

const synth = new DemoSynth(api.audioEnabled);
let latestPublisher;

function formatTime(seconds) {
  if (!Number.isFinite(seconds)) return "--:--";
  const value = Math.max(0, Math.round(seconds));
  const minutes = Math.floor(value / 60);
  return `${minutes}:${String(value % 60).padStart(2, "0")}`;
}

function safeJson(value) {
  return JSON.stringify(
    value,
    (_key, item) => (typeof item === "bigint" ? String(item) : item),
    2,
  );
}

function playbackModeLabel(publisher) {
  const modes = [];
  if (publisher.shuffleEnabled) modes.push("Shuffle");
  if (publisher.autoRepeatMode !== 0) {
    modes.push(REPEAT_MODE_NAMES[publisher.autoRepeatMode]);
  }
  return modes.join(" + ") || "Sequential";
}

function options() {
  return {
    title: element("#title").value,
    artist: element("#artist").value,
    durationSeconds: Number(element("#duration").value),
  };
}

function show(value) {
  element("#status").textContent = safeJson(value);
}

function setConnection(label, state) {
  const status = element("#connection-status");
  status.textContent = label;
  status.className = `status-pill ${state}`;
}

function observe(promise) {
  promise.catch((error) => {
    console.error("SMTC sample operation failed.", error);
  });
}

function renderPlaylist(publisher) {
  const playlist = element("#playlist");
  playlist.replaceChildren();
  for (const track of publisher.playlist) {
    const item = document.createElement("li");
    item.dataset.index = String(track.index + 1).padStart(2, "0");
    if (track.index === publisher.trackIndex) item.classList.add("active");

    const title = document.createElement("span");
    title.textContent = track.title;
    const duration = document.createElement("span");
    duration.className = "track-duration";
    duration.textContent = formatTime(track.durationSeconds);
    item.append(title, duration);
    playlist.append(item);
  }
}

function renderCapabilities(capabilities) {
  const container = element("#capabilities");
  container.replaceChildren();
  const entries = Object.entries(capabilities);
  if (entries.length === 0) {
    const pending = document.createElement("span");
    pending.className = "capability pending";
    pending.textContent = "Waiting for playback info";
    container.append(pending);
  } else {
    for (const [name, enabled] of entries) {
      const chip = document.createElement("span");
      chip.className = `capability ${enabled ? "enabled" : "disabled"}`;
      chip.textContent = `${name}: ${enabled ? "enabled" : "disabled"}`;
      container.append(chip);
    }
  }

  for (const button of document.querySelectorAll("[data-capability]")) {
    const capability = button.dataset.capability;
    button.disabled = !initialized || capabilities[capability] !== true;
  }
}

function renderSnapshot(snapshot) {
  const { publisher, controller } = snapshot;
  const { metadata, playback, timeline, capabilities } = controller;
  latestPublisher = publisher;
  observe(synth.sync(publisher));

  element("#publisher-artwork").src = publisher.artworkUrl;
  element("#publisher-track-number").textContent =
    `Track ${publisher.trackIndex + 1} of ${publisher.trackCount}`;
  element("#publisher-title").textContent = publisher.title;
  element("#publisher-artist").textContent = publisher.artist;
  element("#publisher-album").textContent =
    `${publisher.albumTitle} · ${publisher.genre}`;
  element("#publisher-status").textContent =
    SMTC_STATUS_NAMES[publisher.playbackStatus] ??
    String(publisher.playbackStatus);
  element("#publisher-rate").textContent =
    `${publisher.playbackRate.toFixed(2)}x`;
  element("#publisher-mode").textContent = playbackModeLabel(publisher);
  const isPlaying = publisher.playbackStatus === 3;
  const toggleButton = element('[data-action="toggle"]');
  toggleButton.setAttribute("aria-label", isPlaying ? "Pause" : "Play");
  toggleButton.title = isPlaying ? "Pause" : "Play";
  element("#toggle-icon-path").setAttribute(
    "d",
    isPlaying ? "M5.5 4h3v12h-3V4Zm6 0h3v12h-3V4Z" : "M6 3.8v12.4L16 10 6 3.8Z",
  );

  const publisherProgress = element("#publisher-progress");
  publisherProgress.max = String(publisher.durationSeconds);
  publisherProgress.value = String(publisher.positionSeconds);
  element("#publisher-position").textContent = formatTime(
    publisher.positionSeconds,
  );
  element("#publisher-duration").textContent = formatTime(
    publisher.durationSeconds,
  );
  renderPlaylist(publisher);

  const trackKey = `${publisher.trackIndex}\0${publisher.title}`;
  if (
    trackKey !== renderedTrackKey &&
    !["title", "artist", "duration"].includes(document.activeElement?.id)
  ) {
    element("#title").value = publisher.title;
    element("#artist").value = publisher.artist;
    element("#duration").value = String(publisher.durationSeconds);
    renderedTrackKey = trackKey;
  }

  element("#controller-artwork").src = publisher.artworkUrl;
  element("#controller-artwork").hidden = !metadata.hasThumbnail;
  element("#controller-title").textContent = metadata.title || "No media title";
  element("#controller-artist").textContent = [
    metadata.artist,
    metadata.albumTitle,
  ]
    .filter(Boolean)
    .join(" - ");
  element("#controller-source").textContent =
    controller.sourceAppUserModelId || "No source AUMID";

  const seek = element("#seek");
  seek.max = String(timeline.endSeconds ?? publisher.durationSeconds);
  if (document.activeElement !== seek) {
    seek.value = String(timeline.positionSeconds ?? 0);
    element("#seek-value").textContent = formatTime(
      timeline.positionSeconds ?? 0,
    );
  }

  const rate = element("#rate");
  const rateValue = String(playback.rate ?? publisher.playbackRate);
  if ([...rate.options].some((option) => option.value === rateValue)) {
    rate.value = rateValue;
  }
  element("#shuffle").checked =
    playback.shuffleActive ?? publisher.shuffleEnabled;
  element("#repeat").value = String(
    playback.autoRepeatMode ?? publisher.autoRepeatMode,
  );
  renderCapabilities(capabilities);

  setConnection(
    `${GSMTC_STATUS_NAMES[playback.status] ?? "Connected"} via GSMTC`,
    "active",
  );
}

function renderSessions(sessions) {
  const container = element("#sessions");
  container.replaceChildren();
  if (sessions.length === 0) {
    const empty = document.createElement("p");
    empty.className = "empty-state";
    empty.textContent = "No global media sessions are currently available.";
    container.append(empty);
    return;
  }

  for (const session of sessions) {
    const card = document.createElement("article");
    card.className = "session-card";
    if (session.isOwn) card.classList.add("own");
    if (session.isCurrent) card.classList.add("current");

    const title = document.createElement("h3");
    title.textContent = session.title || "Untitled media session";
    const artist = document.createElement("p");
    artist.textContent = session.artist || "Unknown artist";
    const source = document.createElement("p");
    source.textContent = session.sourceAppUserModelId || session.error || "";
    const badges = document.createElement("div");
    badges.className = "session-badges";
    if (session.isOwn) {
      const own = document.createElement("span");
      own.textContent = "This sample";
      badges.append(own);
    }
    if (session.isCurrent) {
      const current = document.createElement("span");
      current.textContent = "Windows current";
      badges.append(current);
    }
    const status = document.createElement("span");
    status.textContent =
      GSMTC_STATUS_NAMES[session.playbackStatus] ?? "Unavailable";
    badges.append(status);
    card.append(title, artist, source, badges);
    container.append(card);
  }
}

function renderEvents() {
  const list = element("#events");
  list.replaceChildren();
  if (eventLines.length === 0) {
    const empty = document.createElement("li");
    empty.className = "empty-state";
    empty.textContent = "Waiting for SMTC and GSMTC events.";
    list.append(empty);
    return;
  }

  for (const event of eventLines) {
    const item = document.createElement("li");
    item.className = "event-item";
    if (event.flow?.startsWith("GSMTC controller")) {
      item.classList.add("controller");
    } else if (event.flow?.includes("manager")) {
      item.classList.add("manager");
    }

    const time = document.createElement("span");
    time.className = "event-time";
    time.textContent = new Date(event.timestamp).toLocaleTimeString();
    const type = document.createElement("span");
    type.className = "event-type";
    type.textContent = event.type;
    const flow = document.createElement("span");
    flow.className = "event-flow";
    flow.textContent = event.flow ?? "Loopback";
    const detail = document.createElement("span");
    detail.className = "event-detail";
    const detailValue = { ...event };
    delete detailValue.type;
    delete detailValue.timestamp;
    delete detailValue.flow;
    detail.textContent =
      Object.keys(detailValue).length > 0 ? safeJson(detailValue) : "";
    item.append(time, type, flow, detail);
    list.append(item);
  }
}

async function run(action) {
  try {
    return await action();
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    setConnection("Operation failed", "error");
    show({ error: message });
    throw error;
  }
}

async function refreshSnapshot() {
  if (!initialized || snapshotInFlight) return;
  snapshotInFlight = true;
  try {
    renderSnapshot(await api.snapshot());
  } catch (error) {
    setConnection("Snapshot unavailable", "error");
    show({ error: error instanceof Error ? error.message : String(error) });
  } finally {
    snapshotInFlight = false;
  }
}

async function refreshSessions() {
  if (!initialized || sessionsInFlight) return;
  sessionsInFlight = true;
  try {
    renderSessions(await api.sessions());
  } catch (error) {
    show({ error: error instanceof Error ? error.message : String(error) });
  } finally {
    sessionsInFlight = false;
  }
}

function scheduleRefresh() {
  if (refreshTimer) return;
  refreshTimer = setTimeout(() => {
    refreshTimer = undefined;
    void refreshSnapshot();
  }, 120);
}

async function initialize() {
  setConnection("Starting native session", "idle");
  const snapshot = await run(() => api.initialize(options()));
  initialized = true;
  element("#initialize").textContent = "Restart media session";
  renderSnapshot(snapshot);
  await refreshSessions();
  return snapshot;
}

async function sendControl(action, value) {
  const accepted = await run(() => api.control(action, value));
  scheduleRefresh();
  return accepted;
}

element("#initialize").addEventListener("click", () => {
  observe(
    (async () => {
      await synth.unlock();
      return initialize();
    })(),
  );
});

element("#update").addEventListener("click", () => {
  observe(
    run(async () => {
      const snapshot = await api.update(options());
      renderSnapshot(snapshot);
      show({ updated: snapshot.controller.metadata });
    }),
  );
});

for (const button of document.querySelectorAll("[data-action]")) {
  button.disabled = true;
  button.addEventListener("click", () => {
    observe(
      (async () => {
        await synth.unlock();
        return sendControl(button.dataset.action);
      })(),
    );
  });
}

element("#audio-toggle").addEventListener("click", () => {
  observe(synth.toggle());
});

element("#seek").addEventListener("input", (event) => {
  element("#seek-value").textContent = formatTime(Number(event.target.value));
});
element("#seek").addEventListener("change", (event) => {
  observe(sendControl("seek", Number(event.target.value)));
});
element("#rate").addEventListener("change", (event) => {
  observe(sendControl("rate", Number(event.target.value)));
});
element("#shuffle").addEventListener("change", (event) => {
  observe(sendControl("shuffle", event.target.checked));
});
element("#repeat").addEventListener("change", (event) => {
  observe(sendControl("repeat", Number(event.target.value)));
});

element("#refresh-sessions").addEventListener("click", () => {
  void refreshSessions();
});
element("#clear-events").addEventListener("click", () => {
  eventLines.length = 0;
  renderEvents();
});

element("#validate").addEventListener("click", () => {
  observe(
    run(async () => {
      setConnection("Running loopback validation", "idle");
      const result = await api.validate();
      initialized = true;
      element("#initialize").textContent = "Restart media session";
      renderSnapshot(result.snapshot);
      renderSessions(result.sessions);
      show(result);
      const failures = Object.values(result.checks).filter(
        (passed) => !passed,
      ).length;
      setConnection(
        failures === 0
          ? `${Object.keys(result.checks).length} checks passed`
          : `${failures} checks failed`,
        failures === 0 ? "active" : "error",
      );
    }),
  );
});

const unsubscribe = api.onEvent((event) => {
  eventLines.unshift(event);
  eventLines.splice(60);
  renderEvents();
  scheduleRefresh();
  if (
    event.type === "sessions-changed" ||
    event.type === "current-session-changed"
  ) {
    void refreshSessions();
  }
});

window.showValidationResult = (result) => {
  if (result.error) {
    setConnection("Validation failed", "error");
    show(result);
    return;
  }
  initialized = true;
  element("#initialize").textContent = "Restart media session";
  renderSnapshot(result.snapshot);
  renderSessions(result.sessions ?? []);
  show(result);
  setConnection(`${Object.keys(result.checks).length} checks passed`, "active");
};

const snapshotInterval = setInterval(() => {
  void refreshSnapshot();
}, 1000);
const sessionsInterval = setInterval(() => {
  void refreshSessions();
}, 5000);
window.addEventListener("beforeunload", () => {
  initialized = false;
  clearInterval(snapshotInterval);
  clearInterval(sessionsInterval);
  unsubscribe();
  synth.dispose();
});
