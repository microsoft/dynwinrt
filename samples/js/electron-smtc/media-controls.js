// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

const path = require("node:path");
const {
  GlobalSystemMediaTransportControlsSessionManager,
} = require("./generated/GlobalSystemMediaTransportControlsSessionManager.js");
const {
  MediaPlaybackAutoRepeatMode,
} = require("./generated/MediaPlaybackAutoRepeatMode.js");
const { MediaPlaybackStatus } = require("./generated/MediaPlaybackStatus.js");
const { MediaPlaybackType } = require("./generated/MediaPlaybackType.js");
const {
  RandomAccessStreamReference,
} = require("./generated/RandomAccessStreamReference.js");
const { StorageFile } = require("./generated/StorageFile.js");
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
const { releaseProjected } = require("./generated/lifetime.js");

const TICKS_PER_SECOND = 10_000_000n;
const TIMELINE_INTERVAL_MS = 500;
const SEEK_STEP_SECONDS = 15;

const DEFAULT_PLAYLIST = Object.freeze([
  Object.freeze({
    title: "Neon Window",
    artist: "dynwinrt Ensemble",
    albumTitle: "Runtime Radio",
    genre: "Electronic",
    durationSeconds: 185,
    artworkUrl: "./Assets/AlbumBlue.png",
  }),
  Object.freeze({
    title: "Metadata Drive",
    artist: "The Interface Band",
    albumTitle: "Runtime Radio",
    genre: "Synthwave",
    durationSeconds: 214,
    artworkUrl: "./Assets/AlbumPurple.png",
  }),
  Object.freeze({
    title: "Interop Nights",
    artist: "COM & The Projections",
    albumTitle: "Runtime Radio",
    genre: "Ambient",
    durationSeconds: 168,
    artworkUrl: "./Assets/AlbumOrange.png",
  }),
]);

const BUTTON_NAMES = new Map(
  Object.entries(SystemMediaTransportControlsButton).map(([name, value]) => [
    value,
    name,
  ]),
);

function secondsToTicks(seconds) {
  return BigInt(Math.round(Number(seconds) * Number(TICKS_PER_SECOND)));
}

function ticksToSeconds(ticks) {
  return Number(ticks) / Number(TICKS_PER_SECOND);
}

function sleep(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function errorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}

function finiteNumber(value, name) {
  const number = Number(value);
  if (!Number.isFinite(number)) {
    throw new TypeError(`${name} must be a finite number.`);
  }
  return number;
}

function releaseProjectedValues(...values) {
  let firstError;
  for (const value of values) {
    if (value == null) continue;
    try {
      releaseProjected(value);
    } catch (error) {
      firstError ??= error;
    }
  }
  if (firstError) throw firstError;
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
    this.artworkReferences = [];
    this.unsubscribers = [];
    this.timelineTimer = null;
    this.playlist = [];
    this.trackIndex = 0;
    this.durationSeconds = 0;
    this.positionSeconds = 0;
    this.playbackStatus = MediaPlaybackStatus.Stopped;
    this.playbackRate = 1;
    this.shuffleEnabled = false;
    this.autoRepeatMode = MediaPlaybackAutoRepeatMode.None;
    this.lastPositionUpdateAt = Date.now();
    this.recordRequests = 0;
    this.eventCounts = this.#newEventCounts();
  }

  #newEventCounts() {
    return {
      buttonPressed: 0,
      propertyChanged: 0,
      positionRequested: 0,
      playbackRateRequested: 0,
      shuffleRequested: 0,
      repeatRequested: 0,
      managerSessionsChanged: 0,
      managerCurrentSessionChanged: 0,
      mediaPropertiesChanged: 0,
      playbackInfoChanged: 0,
      timelinePropertiesChanged: 0,
      trackChanged: 0,
      timelineTicks: 0,
    };
  }

  #currentTrack() {
    return this.playlist[this.trackIndex];
  }

  #emit(type, detail = {}) {
    this.emitEvent({ type, timestamp: new Date().toISOString(), ...detail });
  }

  #releasedEventHandler(callback) {
    return (sender, args) => {
      try {
        callback(args);
      } finally {
        releaseProjectedValues(args, sender);
      }
    };
  }

  async #loadArtworkReferences() {
    const references = [];
    try {
      for (const track of this.playlist) {
        let file;
        try {
          const artworkPath = path.join(__dirname, track.artworkUrl);
          file = await StorageFile.getFileFromPathAsync(artworkPath);
          if (!file) {
            throw new Error(`Artwork file was not found: ${artworkPath}`);
          }
          const reference = RandomAccessStreamReference.createFromFile(file);
          if (!reference) {
            throw new Error(
              `Could not create a stream reference for ${artworkPath}.`,
            );
          }
          references.push(reference);
        } finally {
          releaseProjectedValues(file);
        }
      }
      this.artworkReferences = references;
    } catch (error) {
      releaseProjectedValues(...references);
      throw error;
    }
  }

  #updateMetadata() {
    const track = this.#currentTrack();
    const updater = this.smtc.displayUpdater;
    let musicProperties;
    let genres;
    try {
      updater.type = MediaPlaybackType.Music;
      updater.appMediaId = `dynwinrt-electron-smtc-${this.trackIndex + 1}`;
      updater.thumbnail = this.artworkReferences[this.trackIndex];
      musicProperties = updater.musicProperties;
      musicProperties.title = track.title;
      musicProperties.artist = track.artist;
      musicProperties.albumArtist = "dynwinrt Samples";
      musicProperties.albumTitle = track.albumTitle;
      musicProperties.trackNumber = this.trackIndex + 1;
      musicProperties.albumTrackCount = this.playlist.length;
      genres = musicProperties.genres;
      genres.clear();
      genres.append(track.genre);
      updater.update();
    } finally {
      releaseProjectedValues(genres, musicProperties, updater);
    }
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

  #captureElapsedPosition() {
    const now = Date.now();
    if (this.playbackStatus === MediaPlaybackStatus.Playing) {
      const elapsedSeconds = (now - this.lastPositionUpdateAt) / 1000;
      this.positionSeconds = Math.min(
        this.durationSeconds,
        this.positionSeconds + elapsedSeconds * this.playbackRate,
      );
    }
    this.lastPositionUpdateAt = now;
  }

  #setPlaybackStatus(status, resetPosition = false) {
    this.#captureElapsedPosition();
    if (resetPosition) this.positionSeconds = 0;
    this.playbackStatus = status;
    this.smtc.playbackStatus = status;
    this.lastPositionUpdateAt = Date.now();
    this.#updateTimeline();
    this.#emit("publisher-state-changed", {
      flow: "SMTC publisher",
      playbackStatus: status,
      positionSeconds: this.positionSeconds,
    });
  }

  #seekTo(positionSeconds, reason) {
    this.positionSeconds = Math.max(
      0,
      Math.min(
        this.durationSeconds,
        finiteNumber(positionSeconds, "Playback position"),
      ),
    );
    this.lastPositionUpdateAt = Date.now();
    this.#updateTimeline();
    this.#emit("position-changed", {
      flow: "SMTC publisher",
      reason,
      positionSeconds: this.positionSeconds,
    });
  }

  #relativeTrackIndex(offset) {
    const count = this.playlist.length;
    if (this.shuffleEnabled && offset > 0 && count > 1) {
      return (
        (this.trackIndex + 1 + Math.floor(Math.random() * (count - 1))) % count
      );
    }
    return (this.trackIndex + offset + count) % count;
  }

  #selectTrack(index, reason) {
    this.trackIndex = index;
    this.durationSeconds = this.#currentTrack().durationSeconds;
    this.positionSeconds = 0;
    this.lastPositionUpdateAt = Date.now();
    this.#updateMetadata();
    this.#updateTimeline();
    this.eventCounts.trackChanged += 1;
    this.#emit("track-changed", {
      flow: "SMTC publisher -> GSMTC",
      reason,
      trackIndex: this.trackIndex,
      title: this.#currentTrack().title,
    });
  }

  #selectRelativeTrack(offset, reason) {
    this.#selectTrack(this.#relativeTrackIndex(offset), reason);
  }

  #advanceTimeline() {
    if (!this.smtc) return;
    if (this.playbackStatus !== MediaPlaybackStatus.Playing) {
      this.lastPositionUpdateAt = Date.now();
      return;
    }

    this.#captureElapsedPosition();
    if (this.positionSeconds >= this.durationSeconds) {
      if (this.autoRepeatMode === MediaPlaybackAutoRepeatMode.Track) {
        this.#seekTo(0, "repeat-track");
      } else if (
        this.shuffleEnabled ||
        this.trackIndex < this.playlist.length - 1 ||
        this.autoRepeatMode === MediaPlaybackAutoRepeatMode.List
      ) {
        this.#selectRelativeTrack(1, "automatic-next");
      } else {
        this.positionSeconds = this.durationSeconds;
        this.playbackStatus = MediaPlaybackStatus.Stopped;
        this.smtc.playbackStatus = MediaPlaybackStatus.Stopped;
        this.#updateTimeline();
      }
    } else {
      this.#updateTimeline();
    }

    this.eventCounts.timelineTicks += 1;
    this.#emit("timeline-tick", {
      flow: "SMTC publisher -> GSMTC",
      positionSeconds: this.positionSeconds,
      durationSeconds: this.durationSeconds,
    });
  }

  #startTimelineTimer() {
    this.timelineTimer = setInterval(() => {
      try {
        this.#advanceTimeline();
      } catch (error) {
        this.#emit("timeline-error", {
          flow: "SMTC publisher",
          error: errorMessage(error),
        });
      }
    }, TIMELINE_INTERVAL_MS);
  }

  #handleButton(button) {
    switch (button) {
      case SystemMediaTransportControlsButton.Play:
        this.#setPlaybackStatus(MediaPlaybackStatus.Playing);
        break;
      case SystemMediaTransportControlsButton.Pause:
        this.#setPlaybackStatus(MediaPlaybackStatus.Paused);
        break;
      case SystemMediaTransportControlsButton.Stop:
        this.#setPlaybackStatus(MediaPlaybackStatus.Stopped, true);
        break;
      case SystemMediaTransportControlsButton.Next:
      case SystemMediaTransportControlsButton.ChannelUp:
        this.#selectRelativeTrack(1, BUTTON_NAMES.get(button));
        break;
      case SystemMediaTransportControlsButton.Previous:
      case SystemMediaTransportControlsButton.ChannelDown:
        this.#selectRelativeTrack(-1, BUTTON_NAMES.get(button));
        break;
      case SystemMediaTransportControlsButton.FastForward:
        this.#seekTo(this.positionSeconds + SEEK_STEP_SECONDS, "fast-forward");
        break;
      case SystemMediaTransportControlsButton.Rewind:
        this.#seekTo(this.positionSeconds - SEEK_STEP_SECONDS, "rewind");
        break;
      case SystemMediaTransportControlsButton.Record:
        this.recordRequests += 1;
        break;
      default:
        throw new Error(`Unsupported SMTC button value: ${button}`);
    }

    this.#emit("button-pressed", {
      flow: "GSMTC controller -> SMTC",
      button,
      buttonName: BUTTON_NAMES.get(button) ?? String(button),
    });
  }

  #subscribeManagerEvents() {
    this.unsubscribers.push(
      this.manager.onSessionsChanged(
        this.#releasedEventHandler(() => {
          this.eventCounts.managerSessionsChanged += 1;
          this.#emit("sessions-changed", { flow: "GSMTC manager" });
        }),
      ),
      this.manager.onCurrentSessionChanged(
        this.#releasedEventHandler(() => {
          this.eventCounts.managerCurrentSessionChanged += 1;
          this.#emit("current-session-changed", { flow: "GSMTC manager" });
        }),
      ),
    );
  }

  #subscribeSmtcEvents() {
    this.unsubscribers.push(
      this.smtc.onButtonPressed(
        this.#releasedEventHandler((args) => {
          this.eventCounts.buttonPressed += 1;
          this.#handleButton(args.button);
        }),
      ),
      this.smtc.onPropertyChanged(
        this.#releasedEventHandler((args) => {
          this.eventCounts.propertyChanged += 1;
          this.#emit("smtc-property-changed", {
            flow: "Windows -> SMTC publisher",
            property: args.property,
            soundLevel: this.smtc.soundLevel,
          });
        }),
      ),
      this.smtc.onPlaybackPositionChangeRequested(
        this.#releasedEventHandler((args) => {
          this.eventCounts.positionRequested += 1;
          this.#seekTo(
            ticksToSeconds(args.requestedPlaybackPosition.duration),
            "GSMTC seek request",
          );
        }),
      ),
      this.smtc.onPlaybackRateChangeRequested(
        this.#releasedEventHandler((args) => {
          this.eventCounts.playbackRateRequested += 1;
          this.#captureElapsedPosition();
          this.playbackRate = args.requestedPlaybackRate;
          this.smtc.playbackRate = this.playbackRate;
          this.#updateTimeline();
          this.#emit("playback-rate-requested", {
            flow: "GSMTC controller -> SMTC",
            playbackRate: this.playbackRate,
          });
        }),
      ),
      this.smtc.onShuffleEnabledChangeRequested(
        this.#releasedEventHandler((args) => {
          this.eventCounts.shuffleRequested += 1;
          this.shuffleEnabled = args.requestedShuffleEnabled;
          this.smtc.shuffleEnabled = this.shuffleEnabled;
          this.#emit("shuffle-requested", {
            flow: "GSMTC controller -> SMTC",
            enabled: this.shuffleEnabled,
          });
        }),
      ),
      this.smtc.onAutoRepeatModeChangeRequested(
        this.#releasedEventHandler((args) => {
          this.eventCounts.repeatRequested += 1;
          this.autoRepeatMode = args.requestedAutoRepeatMode;
          this.smtc.autoRepeatMode = this.autoRepeatMode;
          this.#emit("repeat-requested", {
            flow: "GSMTC controller -> SMTC",
            mode: this.autoRepeatMode,
          });
        }),
      ),
    );
  }

  #subscribeSessionEvents() {
    this.unsubscribers.push(
      this.session.onMediaPropertiesChanged(
        this.#releasedEventHandler(() => {
          this.eventCounts.mediaPropertiesChanged += 1;
          this.#emit("media-properties-changed", {
            flow: "SMTC publisher -> GSMTC controller",
          });
        }),
      ),
      this.session.onPlaybackInfoChanged(
        this.#releasedEventHandler(() => {
          this.eventCounts.playbackInfoChanged += 1;
          this.#emit("playback-info-changed", {
            flow: "SMTC publisher -> GSMTC controller",
          });
        }),
      ),
      this.session.onTimelinePropertiesChanged(
        this.#releasedEventHandler(() => {
          this.eventCounts.timelinePropertiesChanged += 1;
          this.#emit("timeline-properties-changed", {
            flow: "SMTC publisher -> GSMTC controller",
          });
        }),
      ),
    );
  }

  async #findOwnSession() {
    const deadline = Date.now() + 10_000;
    while (Date.now() < deadline) {
      const sessions = this.manager.getSessions();
      if (!sessions) {
        await sleep(100);
        continue;
      }

      const candidates = sessions.toArray();
      let match = null;
      try {
        for (const candidate of candidates) {
          let properties;
          try {
            properties = await candidate.tryGetMediaPropertiesAsync();
            if (properties?.title === this.#currentTrack().title) {
              match = candidate;
              break;
            }
          } catch (error) {
            this.#emit("session-read-retry", {
              flow: "GSMTC manager",
              error: errorMessage(error),
            });
          } finally {
            releaseProjectedValues(properties);
          }
        }
      } finally {
        releaseProjectedValues(
          ...candidates.filter((candidate) => candidate !== match),
          sessions,
        );
      }

      if (match) return match;
      await sleep(100);
    }
    throw new Error(
      "Timed out waiting for the Electron SMTC session in GSMTC.",
    );
  }

  #publisherState() {
    const track = this.#currentTrack();
    return {
      trackIndex: this.trackIndex,
      trackCount: this.playlist.length,
      title: track.title,
      artist: track.artist,
      albumTitle: track.albumTitle,
      genre: track.genre,
      artworkUrl: track.artworkUrl,
      durationSeconds: this.durationSeconds,
      positionSeconds: this.positionSeconds,
      playbackStatus: this.playbackStatus,
      playbackRate: this.playbackRate,
      shuffleEnabled: this.shuffleEnabled,
      autoRepeatMode: this.autoRepeatMode,
      soundLevel: this.smtc?.soundLevel ?? null,
      recordRequests: this.recordRequests,
      playlist: this.playlist.map((item, index) => ({
        index,
        title: item.title,
        artist: item.artist,
        durationSeconds: item.durationSeconds,
        artworkUrl: item.artworkUrl,
      })),
    };
  }

  async initialize(options = {}) {
    this.dispose();
    this.eventCounts = this.#newEventCounts();
    this.playlist = DEFAULT_PLAYLIST.map((track) => ({ ...track }));
    this.trackIndex = 0;

    const track = this.#currentTrack();
    if (options.title?.trim()) track.title = options.title.trim();
    if (options.artist?.trim()) track.artist = options.artist.trim();
    if (options.durationSeconds != null) {
      track.durationSeconds = Math.max(
        1,
        finiteNumber(options.durationSeconds, "Duration"),
      );
    }

    this.durationSeconds = track.durationSeconds;
    this.positionSeconds = Math.max(
      0,
      Math.min(
        this.durationSeconds,
        options.positionSeconds == null
          ? 0
          : finiteNumber(options.positionSeconds, "Playback position"),
      ),
    );
    this.playbackStatus = MediaPlaybackStatus.Playing;
    this.playbackRate = 1;
    this.shuffleEnabled = false;
    this.autoRepeatMode = MediaPlaybackAutoRepeatMode.None;
    this.recordRequests = 0;
    this.lastPositionUpdateAt = Date.now();

    try {
      await this.#loadArtworkReferences();
      this.manager =
        await GlobalSystemMediaTransportControlsSessionManager.requestAsync();
      if (!this.manager) {
        throw new Error("GSMTC session manager initialization returned null.");
      }
      this.#subscribeManagerEvents();

      const interop = ISystemMediaTransportControlsInterop.create();
      try {
        const raw = interop.getForWindow(this.window.getNativeWindowHandle());
        this.smtc = SystemMediaTransportControls._fromNative(raw);
      } finally {
        interop.release();
      }

      this.timeline = new SystemMediaTransportControlsTimelineProperties();
      this.smtc.isPlayEnabled = true;
      this.smtc.isPauseEnabled = true;
      this.smtc.isStopEnabled = true;
      this.smtc.isRecordEnabled = true;
      this.smtc.isFastForwardEnabled = true;
      this.smtc.isRewindEnabled = true;
      this.smtc.isNextEnabled = true;
      this.smtc.isPreviousEnabled = true;
      this.smtc.isChannelUpEnabled = true;
      this.smtc.isChannelDownEnabled = true;
      this.smtc.playbackStatus = this.playbackStatus;
      this.smtc.playbackRate = this.playbackRate;
      this.smtc.shuffleEnabled = this.shuffleEnabled;
      this.smtc.autoRepeatMode = this.autoRepeatMode;
      this.#subscribeSmtcEvents();
      this.smtc.isEnabled = true;
      this.#updateMetadata();
      this.#updateTimeline();

      this.session = await this.#findOwnSession();
      this.#subscribeSessionEvents();
      this.#startTimelineTimer();
      this.#emit("initialized", {
        flow: "SMTC publisher -> GSMTC manager",
        sourceAppUserModelId: this.session.sourceAppUserModelId,
      });
      return await this.snapshot();
    } catch (error) {
      try {
        this.dispose();
      } catch (cleanupError) {
        throw new AggregateError(
          [error, cleanupError],
          "Media session initialization and cleanup failed.",
        );
      }
      throw error;
    }
  }

  async update(options = {}) {
    if (!this.smtc) throw new Error("Initialize the media session first.");
    const track = this.#currentTrack();
    if (options.title?.trim()) track.title = options.title.trim();
    if (options.artist?.trim()) track.artist = options.artist.trim();
    if (options.durationSeconds != null) {
      track.durationSeconds = Math.max(
        1,
        finiteNumber(options.durationSeconds, "Duration"),
      );
      this.durationSeconds = track.durationSeconds;
      this.positionSeconds = Math.min(
        this.positionSeconds,
        this.durationSeconds,
      );
    }
    this.#updateMetadata();
    this.#updateTimeline();
    this.#emit("metadata-updated", {
      flow: "SMTC publisher -> GSMTC controller",
      title: track.title,
    });
    return this.snapshot();
  }

  async control(action, value) {
    if (!this.session) throw new Error("Initialize the media session first.");
    switch (action) {
      case "play":
        return this.session.tryPlayAsync();
      case "pause":
        return this.session.tryPauseAsync();
      case "toggle":
        return this.session.tryTogglePlayPauseAsync();
      case "stop":
        return this.session.tryStopAsync();
      case "record":
        return this.session.tryRecordAsync();
      case "fastForward":
        return this.session.tryFastForwardAsync();
      case "rewind":
        return this.session.tryRewindAsync();
      case "next":
        return this.session.trySkipNextAsync();
      case "previous":
        return this.session.trySkipPreviousAsync();
      case "channelUp":
        return this.session.tryChangeChannelUpAsync();
      case "channelDown":
        return this.session.tryChangeChannelDownAsync();
      case "seek":
        return this.session.tryChangePlaybackPositionAsync(
          secondsToTicks(finiteNumber(value, "Playback position")),
        );
      case "rate":
        return this.session.tryChangePlaybackRateAsync(
          finiteNumber(value, "Playback rate"),
        );
      case "shuffle":
        return this.session.tryChangeShuffleActiveAsync(Boolean(value));
      case "repeat":
        return this.session.tryChangeAutoRepeatModeAsync(
          finiteNumber(value, "Repeat mode"),
        );
      default:
        throw new Error(`Unknown media-control action: ${action}`);
    }
  }

  async snapshot() {
    if (!this.session) throw new Error("Initialize the media session first.");
    let properties;
    let genres;
    let thumbnail;
    let playback;
    let controls;
    let timeline;
    try {
      properties = await this.session.tryGetMediaPropertiesAsync();
      genres = properties?.genres;
      thumbnail = properties?.thumbnail;
      playback = this.session.getPlaybackInfo();
      controls = playback?.controls;
      timeline = this.session.getTimelineProperties();

      return {
        publisher: this.#publisherState(),
        controller: {
          sourceAppUserModelId: this.session.sourceAppUserModelId,
          metadata: {
            title: properties?.title ?? "",
            subtitle: properties?.subtitle ?? "",
            albumArtist: properties?.albumArtist ?? "",
            artist: properties?.artist ?? "",
            albumTitle: properties?.albumTitle ?? "",
            trackNumber: properties?.trackNumber ?? null,
            albumTrackCount: properties?.albumTrackCount ?? null,
            genres: genres?.toArray() ?? [],
            hasThumbnail: thumbnail != null,
          },
          playback: {
            status: playback?.playbackStatus ?? null,
            rate: playback?.playbackRate ?? null,
            shuffleActive: playback?.isShuffleActive ?? null,
            autoRepeatMode: playback?.autoRepeatMode ?? null,
          },
          timeline: {
            startSeconds: timeline
              ? ticksToSeconds(timeline.startTime.duration)
              : null,
            endSeconds: timeline
              ? ticksToSeconds(timeline.endTime.duration)
              : null,
            minSeekSeconds: timeline
              ? ticksToSeconds(timeline.minSeekTime.duration)
              : null,
            maxSeekSeconds: timeline
              ? ticksToSeconds(timeline.maxSeekTime.duration)
              : null,
            positionSeconds: timeline
              ? ticksToSeconds(timeline.position.duration)
              : null,
            lastUpdatedTime: timeline
              ? String(timeline.lastUpdatedTime.universalTime)
              : null,
          },
          capabilities: controls
            ? {
                play: controls.isPlayEnabled,
                pause: controls.isPauseEnabled,
                stop: controls.isStopEnabled,
                record: controls.isRecordEnabled,
                fastForward: controls.isFastForwardEnabled,
                rewind: controls.isRewindEnabled,
                next: controls.isNextEnabled,
                previous: controls.isPreviousEnabled,
                channelUp: controls.isChannelUpEnabled,
                channelDown: controls.isChannelDownEnabled,
                toggle: controls.isPlayPauseToggleEnabled,
                shuffle: controls.isShuffleEnabled,
                repeat: controls.isRepeatEnabled,
                rate: controls.isPlaybackRateEnabled,
                seek: controls.isPlaybackPositionEnabled,
              }
            : {},
        },
        eventCounts: { ...this.eventCounts },
      };
    } finally {
      releaseProjectedValues(
        timeline,
        controls,
        playback,
        thumbnail,
        genres,
        properties,
      );
    }
  }

  async #readSessionSummary(candidate, currentKey, ownKey) {
    let properties;
    let playback;
    try {
      properties = await candidate.tryGetMediaPropertiesAsync();
      playback = candidate.getPlaybackInfo();
      const sourceAppUserModelId = candidate.sourceAppUserModelId;
      const title = properties?.title ?? "";
      const key = `${sourceAppUserModelId}\0${title}`;
      return {
        sourceAppUserModelId,
        title,
        artist: properties?.artist ?? "",
        playbackStatus: playback?.playbackStatus ?? null,
        isCurrent: key === currentKey,
        isOwn: key === ownKey,
      };
    } finally {
      releaseProjectedValues(playback, properties);
    }
  }

  async listSessions() {
    if (!this.manager || !this.session) {
      throw new Error("Initialize the media session first.");
    }

    let currentSession;
    let currentProperties;
    let sessions;
    try {
      currentSession = this.manager.getCurrentSession();
      currentProperties = await currentSession?.tryGetMediaPropertiesAsync();
      const currentKey = currentSession
        ? `${currentSession.sourceAppUserModelId}\0${
            currentProperties?.title ?? ""
          }`
        : null;
      const ownKey = `${this.session.sourceAppUserModelId}\0${
        this.#currentTrack().title
      }`;

      sessions = this.manager.getSessions();
      if (!sessions) return [];
      const candidates = sessions.toArray();
      const summaries = [];
      try {
        for (const candidate of candidates) {
          try {
            summaries.push(
              await this.#readSessionSummary(candidate, currentKey, ownKey),
            );
          } catch (error) {
            summaries.push({
              sourceAppUserModelId: "",
              title: "Session became unavailable",
              artist: "",
              playbackStatus: null,
              isCurrent: false,
              isOwn: false,
              error: errorMessage(error),
            });
          }
        }
      } finally {
        releaseProjectedValues(...candidates);
      }
      return summaries.sort(
        (left, right) =>
          Number(right.isOwn) - Number(left.isOwn) ||
          Number(right.isCurrent) - Number(left.isCurrent),
      );
    } finally {
      releaseProjectedValues(sessions, currentProperties, currentSession);
    }
  }

  async validate() {
    await this.initialize({
      title: "dynwinrt automated SMTC",
      artist: "dynwinrt",
      durationSeconds: 180,
      positionSeconds: 10,
    });

    const liveStart = this.positionSeconds;
    await waitUntil(
      () => this.positionSeconds > liveStart + 0.2,
      "Live timeline did not advance while playing.",
      2500,
    );
    const liveTimelineAdvanced = this.positionSeconds > liveStart;

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

    const originalTrack = this.trackIndex;
    const nextAccepted = await runRequest(
      "buttonPressed",
      "next",
      undefined,
      "Next did not trigger ButtonPressed.",
    );
    const nextChangedTrack = this.trackIndex !== originalTrack;
    const previousAccepted = await runRequest(
      "buttonPressed",
      "previous",
      undefined,
      "Previous did not trigger ButtonPressed.",
    );
    const previousRestoredTrack = this.trackIndex === originalTrack;

    const recordAccepted = await runRequest(
      "buttonPressed",
      "record",
      undefined,
      "Record did not trigger ButtonPressed.",
    );
    const beforeFastForward = this.positionSeconds;
    const fastForwardAccepted = await runRequest(
      "buttonPressed",
      "fastForward",
      undefined,
      "Fast-forward did not trigger ButtonPressed.",
    );
    const fastForwardMoved =
      this.positionSeconds >= beforeFastForward + SEEK_STEP_SECONDS - 0.1;
    const rewindAccepted = await runRequest(
      "buttonPressed",
      "rewind",
      undefined,
      "Rewind did not trigger ButtonPressed.",
    );
    const rewindMovedBack =
      Math.abs(this.positionSeconds - beforeFastForward) < 0.5;

    const channelUpAccepted = await runRequest(
      "buttonPressed",
      "channelUp",
      undefined,
      "Channel up did not trigger ButtonPressed.",
    );
    const channelDownAccepted = await runRequest(
      "buttonPressed",
      "channelDown",
      undefined,
      "Channel down did not trigger ButtonPressed.",
    );
    const channelRoundTrip = this.trackIndex === originalTrack;
    const toggleAccepted = await runRequest(
      "buttonPressed",
      "toggle",
      undefined,
      "Play/pause toggle did not trigger ButtonPressed.",
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
    this.#currentTrack().title = "dynwinrt automated SMTC validated";
    this.#updateMetadata();
    await waitUntil(
      () => this.eventCounts.mediaPropertiesChanged > mediaBefore,
      "DisplayUpdater.Update did not trigger MediaPropertiesChanged.",
    );

    let snapshot;
    await waitUntil(
      async () => {
        snapshot = await this.snapshot();
        return snapshot.controller.metadata.hasThumbnail;
      },
      "GSMTC did not expose the published artwork.",
      10_000,
    );
    const sessions = await this.listSessions();
    const { controller } = snapshot;
    const capabilities = controller.capabilities;
    const checks = {
      pauseAccepted,
      playAccepted,
      nextAccepted,
      previousAccepted,
      recordAccepted,
      fastForwardAccepted,
      rewindAccepted,
      channelUpAccepted,
      channelDownAccepted,
      toggleAccepted,
      seekAccepted,
      rateAccepted,
      shuffleAccepted,
      repeatAccepted,
      liveTimelineAdvanced,
      nextChangedTrack,
      previousRestoredTrack,
      fastForwardMoved,
      rewindMovedBack,
      channelRoundTrip,
      managerObservedSession:
        this.eventCounts.managerSessionsChanged > 0 ||
        this.eventCounts.managerCurrentSessionChanged > 0,
      ownSessionListed: sessions.some((item) => item.isOwn),
      titleRoundTrip: controller.metadata.title === this.#currentTrack().title,
      artworkRoundTrip: controller.metadata.hasThumbnail,
      genreRoundTrip: controller.metadata.genres.includes(
        this.#currentTrack().genre,
      ),
      positionRoundTrip:
        Math.abs(controller.timeline.positionSeconds - 42) < 0.1,
      playbackRateRoundTrip: controller.playback.rate === 1.25,
      shuffleRoundTrip: controller.playback.shuffleActive === true,
      repeatRoundTrip:
        controller.playback.autoRepeatMode ===
        MediaPlaybackAutoRepeatMode.Track,
      coreCapabilities:
        capabilities.play &&
        capabilities.stop &&
        capabilities.next &&
        capabilities.previous &&
        capabilities.seek &&
        (capabilities.play || capabilities.pause),
      advancedCapabilities:
        capabilities.record &&
        capabilities.fastForward &&
        capabilities.rewind &&
        capabilities.channelUp &&
        capabilities.channelDown &&
        capabilities.toggle &&
        capabilities.rate &&
        capabilities.shuffle &&
        capabilities.repeat,
      playbackInfoEvents: this.eventCounts.playbackInfoChanged > 0,
      timelineEvents: this.eventCounts.timelinePropertiesChanged > 0,
    };
    return { checks, snapshot, sessions };
  }

  dispose() {
    let firstError;
    const cleanup = (action) => {
      try {
        action();
      } catch (error) {
        firstError ??= error;
      }
    };

    if (this.timelineTimer) {
      clearInterval(this.timelineTimer);
      this.timelineTimer = null;
    }
    for (const unsubscribe of this.unsubscribers.splice(0).reverse()) {
      cleanup(unsubscribe);
    }
    if (this.smtc) {
      cleanup(() => {
        this.smtc.isEnabled = false;
      });
    }

    for (const value of [
      this.session,
      this.timeline,
      this.smtc,
      this.manager,
      ...this.artworkReferences,
    ]) {
      if (value) cleanup(() => releaseProjected(value));
    }

    this.smtc = null;
    this.manager = null;
    this.session = null;
    this.timeline = null;
    this.artworkReferences = [];
    this.playlist = [];

    if (firstError) throw firstError;
  }
}

module.exports = { MediaControlsLoopback };
