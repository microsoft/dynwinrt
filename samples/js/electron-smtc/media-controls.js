// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

const {
  GlobalSystemMediaTransportControlsSessionManager,
} = require("./generated/GlobalSystemMediaTransportControlsSessionManager.js");
const {
  MediaPlaybackAutoRepeatMode,
} = require("./generated/MediaPlaybackAutoRepeatMode.js");
const { MediaPlaybackStatus } = require("./generated/MediaPlaybackStatus.js");
const { MediaPlaybackType } = require("./generated/MediaPlaybackType.js");
const {
  SystemMediaTransportControls,
} = require("./generated/SystemMediaTransportControls.js");
const {
  SystemMediaTransportControlsButton,
} = require("./generated/SystemMediaTransportControlsButton.js");
const {
  SystemMediaTransportControlsTimelineProperties,
} = require("./generated/SystemMediaTransportControlsTimelineProperties.js");
const {
  ISystemMediaTransportControlsInterop,
} = require("./generated/com/ISystemMediaTransportControlsInterop.js");

const TICKS_PER_SECOND = 10_000_000n;

function secondsToTicks(seconds) {
  return BigInt(Math.round(Number(seconds) * Number(TICKS_PER_SECOND)));
}

function ticksToSeconds(ticks) {
  return Number(ticks) / Number(TICKS_PER_SECOND);
}

function sleep(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

async function waitUntil(predicate, message, timeoutMs = 5000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (await predicate()) return;
    await sleep(50);
  }
  throw new Error(message);
}

class MediaControlsLoopback {
  constructor(window, emitEvent = () => {}) {
    this.window = window;
    this.emitEvent = emitEvent;
    this.smtc = null;
    this.manager = null;
    this.session = null;
    this.timeline = null;
    this.title = "";
    this.artist = "";
    this.durationSeconds = 300;
    this.positionSeconds = 0;
    this.unsubscribers = [];
    this.eventCounts = this.#newEventCounts();
  }

  #newEventCounts() {
    return {
      buttonPressed: 0,
      positionRequested: 0,
      playbackRateRequested: 0,
      shuffleRequested: 0,
      repeatRequested: 0,
      managerSessionsChanged: 0,
      managerCurrentSessionChanged: 0,
      mediaPropertiesChanged: 0,
      playbackInfoChanged: 0,
      timelinePropertiesChanged: 0,
    };
  }

  #emit(type, detail = {}) {
    this.emitEvent({ type, timestamp: new Date().toISOString(), ...detail });
  }

  #updateMetadata() {
    const updater = this.smtc.displayUpdater;
    updater.type = MediaPlaybackType.Music;
    updater.appMediaId = "dynwinrt-electron-smtc";
    updater.musicProperties.title = this.title;
    updater.musicProperties.artist = this.artist;
    updater.musicProperties.albumTitle = "dynwinrt samples";
    updater.musicProperties.trackNumber = 1;
    updater.update();
  }

  #updateTimeline() {
    this.timeline.startTime = { duration: 0n };
    this.timeline.minSeekTime = { duration: 0n };
    this.timeline.endTime = { duration: secondsToTicks(this.durationSeconds) };
    this.timeline.maxSeekTime = {
      duration: secondsToTicks(this.durationSeconds),
    };
    this.timeline.position = { duration: secondsToTicks(this.positionSeconds) };
    this.smtc.updateTimelineProperties(this.timeline);
  }

  #subscribeManagerEvents() {
    this.unsubscribers.push(
      this.manager.onSessionsChanged(() => {
        this.eventCounts.managerSessionsChanged += 1;
        this.#emit("sessions-changed");
      }),
      this.manager.onCurrentSessionChanged(() => {
        this.eventCounts.managerCurrentSessionChanged += 1;
        this.#emit("current-session-changed");
      }),
    );
  }

  #subscribeSmtcEvents() {
    this.unsubscribers.push(
      this.smtc.onButtonPressed((_sender, args) => {
        this.eventCounts.buttonPressed += 1;
        if (args.button === SystemMediaTransportControlsButton.Play) {
          this.smtc.playbackStatus = MediaPlaybackStatus.Playing;
        } else if (args.button === SystemMediaTransportControlsButton.Pause) {
          this.smtc.playbackStatus = MediaPlaybackStatus.Paused;
        } else if (args.button === SystemMediaTransportControlsButton.Stop) {
          this.smtc.playbackStatus = MediaPlaybackStatus.Stopped;
        }
        this.#emit("button-pressed", { button: args.button });
      }),
      this.smtc.onPlaybackPositionChangeRequested((_sender, args) => {
        this.eventCounts.positionRequested += 1;
        this.positionSeconds = ticksToSeconds(
          args.requestedPlaybackPosition.duration,
        );
        this.#updateTimeline();
        this.#emit("position-requested", {
          positionSeconds: this.positionSeconds,
        });
      }),
      this.smtc.onPlaybackRateChangeRequested((_sender, args) => {
        this.eventCounts.playbackRateRequested += 1;
        this.smtc.playbackRate = args.requestedPlaybackRate;
        this.#emit("playback-rate-requested", {
          playbackRate: args.requestedPlaybackRate,
        });
      }),
      this.smtc.onShuffleEnabledChangeRequested((_sender, args) => {
        this.eventCounts.shuffleRequested += 1;
        this.smtc.shuffleEnabled = args.requestedShuffleEnabled;
        this.#emit("shuffle-requested", {
          enabled: args.requestedShuffleEnabled,
        });
      }),
      this.smtc.onAutoRepeatModeChangeRequested((_sender, args) => {
        this.eventCounts.repeatRequested += 1;
        this.smtc.autoRepeatMode = args.requestedAutoRepeatMode;
        this.#emit("repeat-requested", { mode: args.requestedAutoRepeatMode });
      }),
    );
  }

  #subscribeSessionEvents() {
    this.unsubscribers.push(
      this.session.onMediaPropertiesChanged(() => {
        this.eventCounts.mediaPropertiesChanged += 1;
        this.#emit("media-properties-changed");
      }),
      this.session.onPlaybackInfoChanged(() => {
        this.eventCounts.playbackInfoChanged += 1;
        this.#emit("playback-info-changed");
      }),
      this.session.onTimelinePropertiesChanged(() => {
        this.eventCounts.timelinePropertiesChanged += 1;
        this.#emit("timeline-properties-changed");
      }),
    );
  }

  async #findOwnSession() {
    const deadline = Date.now() + 10_000;
    while (Date.now() < deadline) {
      for (const candidate of this.manager.getSessions()?.toArray() ?? []) {
        try {
          const properties = await candidate.tryGetMediaPropertiesAsync();
          if (properties?.title === this.title) return candidate;
        } catch {
          // The session may disappear between enumeration and metadata retrieval.
        }
      }
      await sleep(100);
    }
    throw new Error(
      "Timed out waiting for the Electron SMTC session in GSMTC.",
    );
  }

  async initialize(options = {}) {
    this.dispose();
    this.eventCounts = this.#newEventCounts();
    this.title = options.title?.trim() || `dynwinrt SMTC ${process.pid}`;
    this.artist = options.artist?.trim() || "dynwinrt";
    this.durationSeconds = Math.max(1, Number(options.durationSeconds) || 300);
    this.positionSeconds = Math.max(
      0,
      Math.min(this.durationSeconds, Number(options.positionSeconds) || 0),
    );

    this.manager =
      await GlobalSystemMediaTransportControlsSessionManager.requestAsync();
    this.#subscribeManagerEvents();

    const interop = ISystemMediaTransportControlsInterop.create();
    try {
      const raw = interop.getForWindow(this.window.getNativeWindowHandle());
      this.smtc = SystemMediaTransportControls._fromNative(raw);
    } finally {
      interop.release();
    }

    this.timeline = new SystemMediaTransportControlsTimelineProperties();
    this.smtc.isEnabled = true;
    this.smtc.isPlayEnabled = true;
    this.smtc.isPauseEnabled = true;
    this.smtc.isStopEnabled = true;
    this.smtc.isNextEnabled = true;
    this.smtc.isPreviousEnabled = true;
    this.smtc.playbackStatus = MediaPlaybackStatus.Playing;
    this.#subscribeSmtcEvents();
    this.#updateMetadata();
    this.#updateTimeline();

    this.session = await this.#findOwnSession();
    this.#subscribeSessionEvents();
    this.#emit("initialized", {
      sourceAppUserModelId: this.session.sourceAppUserModelId,
    });
    return this.snapshot();
  }

  async update(options = {}) {
    if (!this.smtc) throw new Error("Initialize the media session first.");
    if (options.title?.trim()) this.title = options.title.trim();
    if (options.artist?.trim()) this.artist = options.artist.trim();
    if (options.durationSeconds != null) {
      this.durationSeconds = Math.max(1, Number(options.durationSeconds));
    }
    if (options.positionSeconds != null) {
      this.positionSeconds = Math.max(
        0,
        Math.min(this.durationSeconds, Number(options.positionSeconds)),
      );
    }
    this.#updateMetadata();
    this.#updateTimeline();
    return this.snapshot();
  }

  async control(action, value) {
    if (!this.session) throw new Error("Initialize the media session first.");
    switch (action) {
      case "play":
        return this.session.tryPlayAsync();
      case "pause":
        return this.session.tryPauseAsync();
      case "stop":
        return this.session.tryStopAsync();
      case "next":
        return this.session.trySkipNextAsync();
      case "previous":
        return this.session.trySkipPreviousAsync();
      case "seek":
        return this.session.tryChangePlaybackPositionAsync(
          secondsToTicks(value),
        );
      case "rate":
        return this.session.tryChangePlaybackRateAsync(Number(value));
      case "shuffle":
        return this.session.tryChangeShuffleActiveAsync(Boolean(value));
      case "repeat":
        return this.session.tryChangeAutoRepeatModeAsync(Number(value));
      default:
        throw new Error(`Unknown media-control action: ${action}`);
    }
  }

  async snapshot() {
    if (!this.session) throw new Error("Initialize the media session first.");
    const properties = await this.session.tryGetMediaPropertiesAsync();
    const playback = this.session.getPlaybackInfo();
    const timeline = this.session.getTimelineProperties();
    return {
      sourceAppUserModelId: this.session.sourceAppUserModelId,
      title: properties?.title ?? "",
      artist: properties?.artist ?? "",
      playbackStatus: playback?.playbackStatus ?? null,
      playbackRate: playback?.playbackRate ?? null,
      shuffleActive: playback?.isShuffleActive ?? null,
      autoRepeatMode: playback?.autoRepeatMode ?? null,
      positionSeconds: timeline
        ? ticksToSeconds(timeline.position.duration)
        : null,
      durationSeconds: timeline
        ? ticksToSeconds(timeline.endTime.duration)
        : null,
      eventCounts: { ...this.eventCounts },
    };
  }

  async validate() {
    await this.initialize({
      title: "dynwinrt automated SMTC",
      artist: "dynwinrt",
      durationSeconds: 180,
      positionSeconds: 10,
    });

    const runRequest = async (counter, action, value, message) => {
      const before = this.eventCounts[counter];
      const accepted = await this.control(action, value);
      await waitUntil(() => this.eventCounts[counter] > before, message);
      return accepted;
    };

    const pauseAccepted = await runRequest(
      "buttonPressed",
      "pause",
      undefined,
      "Pause did not trigger ButtonPressed.",
    );
    const playAccepted = await runRequest(
      "buttonPressed",
      "play",
      undefined,
      "Play did not trigger ButtonPressed.",
    );
    const nextAccepted = await runRequest(
      "buttonPressed",
      "next",
      undefined,
      "Next did not trigger ButtonPressed.",
    );
    const previousAccepted = await runRequest(
      "buttonPressed",
      "previous",
      undefined,
      "Previous did not trigger ButtonPressed.",
    );
    const seekAccepted = await runRequest(
      "positionRequested",
      "seek",
      42,
      "Seek did not trigger PlaybackPositionChangeRequested.",
    );
    const rateAccepted = await runRequest(
      "playbackRateRequested",
      "rate",
      1.25,
      "Rate change did not trigger PlaybackRateChangeRequested.",
    );
    const shuffleAccepted = await runRequest(
      "shuffleRequested",
      "shuffle",
      true,
      "Shuffle did not trigger ShuffleEnabledChangeRequested.",
    );
    const repeatAccepted = await runRequest(
      "repeatRequested",
      "repeat",
      MediaPlaybackAutoRepeatMode.Track,
      "Repeat did not trigger AutoRepeatModeChangeRequested.",
    );

    const mediaBefore = this.eventCounts.mediaPropertiesChanged;
    this.title = "dynwinrt automated SMTC validated";
    this.#updateMetadata();
    await waitUntil(
      () => this.eventCounts.mediaPropertiesChanged > mediaBefore,
      "DisplayUpdater.Update did not trigger MediaPropertiesChanged.",
    );

    const snapshot = await this.snapshot();
    const checks = {
      pauseAccepted,
      playAccepted,
      nextAccepted,
      previousAccepted,
      seekAccepted,
      rateAccepted,
      shuffleAccepted,
      repeatAccepted,
      managerObservedSession:
        this.eventCounts.managerSessionsChanged > 0 ||
        this.eventCounts.managerCurrentSessionChanged > 0,
      titleRoundTrip: snapshot.title === this.title,
      positionRoundTrip: Math.abs(snapshot.positionSeconds - 42) < 0.01,
      playbackRateRoundTrip: snapshot.playbackRate === 1.25,
      shuffleRoundTrip: snapshot.shuffleActive === true,
      repeatRoundTrip:
        snapshot.autoRepeatMode === MediaPlaybackAutoRepeatMode.Track,
      playbackInfoEvents: this.eventCounts.playbackInfoChanged > 0,
      timelineEvents: this.eventCounts.timelinePropertiesChanged > 0,
    };
    return { checks, snapshot };
  }

  dispose() {
    for (const unsubscribe of this.unsubscribers.splice(0).reverse()) {
      try {
        unsubscribe();
      } catch {}
    }
    if (this.smtc) {
      try {
        this.smtc.isEnabled = false;
      } catch {}
    }
    this.smtc = null;
    this.manager = null;
    this.session = null;
    this.timeline = null;
  }
}

module.exports = { MediaControlsLoopback };
