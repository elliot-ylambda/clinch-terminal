import { forwardRef, useCallback, useEffect, useImperativeHandle, useRef } from "react";
import { FitAddon } from "@xterm/addon-fit";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";

import type { TerminalDimensions } from "../generated/types/TerminalDimensions";
import type { TerminalSnapshot } from "../generated/types/TerminalSnapshot";
import { base64ToBytes } from "../protocol/storage";
import type { TerminalBus } from "./bus";

interface Props {
  snapshot?: TerminalSnapshot;
  bus: TerminalBus;
  canResize: boolean;
  /**
   * The Mac's own grid dimensions, set whenever this device is not the one sizing the PTY.
   *
   * A PTY carries exactly one `winsize`, so two screens cannot hold different widths. When the
   * Mac owns that width this surface adopts its exact column count and scales the rendered grid
   * down to fit the phone, rather than fitting its own narrower column count. Letting the local
   * column count diverge is precisely what turns the Mac's absolute cursor positioning into
   * characters piled up against the right edge.
   */
  mirror?: TerminalDimensions;
  onViewport: (columns: number, rows: number) => void;
  onResize: (columns: number, rows: number) => void;
  onData?: (data: string) => void;
  onFocus?: () => void;
  onStreamGap?: () => void;
}

export interface TerminalSurfaceHandle {
  focus: () => void;
  blur: () => void;
  paste: (text: string) => void;
}

export function isSafeTerminalResize(
  columns: number,
  rows: number,
  width: number,
  height: number,
): boolean {
  return columns >= 20 && rows >= 4 && width >= 160 && height >= 72;
}

/**
 * How far to shrink the Mac's grid so its full width fits this screen.
 *
 * Never magnifies: a Mac pane narrower than the phone leaves space beside it rather than
 * inflating the type past the size chosen for this screen. Returns `undefined` when either
 * measurement is unusable, which happens before the first layout.
 */
export function mirrorScale(availableWidth: number, naturalWidth: number): number | undefined {
  if (!(availableWidth > 0) || !(naturalWidth > 0)) return undefined;
  return Math.min(availableWidth / naturalWidth, 1);
}

/**
 * Where to draw the scroll indicator, as percentages of the terminal surface.
 *
 * Returns `undefined` when nothing has scrolled off the top yet, since there is no position to
 * report. The thumb keeps a floor so a very long transcript still leaves something to see.
 */
export function scrollIndicatorGeometry(
  rows: number,
  baseY: number,
  viewportY: number,
): { heightPercent: number; topPercent: number; atBottom: boolean } | undefined {
  const total = baseY + rows;
  if (baseY <= 0 || total <= 0) return undefined;
  const thumb = Math.min(Math.max(rows / total, 0.05), 1);
  return {
    heightPercent: thumb * 100,
    topPercent: (Math.min(viewportY, baseY) / baseY) * (1 - thumb) * 100,
    atBottom: viewportY >= baseY,
  };
}

const SHELL_INTEGRATION_OSC_PREFIX = new Uint8Array([0x1b, 0x5d, 0x31, 0x33, 0x33, 0x3b]);
const PROMPT_START_MARKER = 0x41;
const PROMPT_END_MARKER = 0x42;
const COMMAND_START_MARKER = 0x43;
const COMMAND_END_MARKER = 0x44;
const SAVE_CURSOR = [0x1b, 0x5b, 0x73];
const RESTORE_AND_CLEAR_CURSOR = [0x1b, 0x5b, 0x75, 0x1b, 0x5b, 0x4a];

/**
 * Clinch's built-in prompt is deliberately zero-width to zsh because the Mac paints it beside a
 * separate command grid. Present that prompt on the preceding line in xterm so zsh's command grid
 * still begins at column zero and retains its full PTY width. This keeps partial redraws and long
 * wrapped commands byte-for-byte compatible with a conventional terminal. The marker recognizer
 * is incremental because a WebSocket frame may split an OSC sequence at any byte.
 */
export class ZeroWidthPromptTransformer {
  private prefixIndex = 0;
  private awaitingMarkerKind = false;
  private insideMarker = false;
  private markerKind: number | undefined;
  private markerEscape = false;
  private activePromptRendered = false;

  reset() {
    this.prefixIndex = 0;
    this.awaitingMarkerKind = false;
    this.insideMarker = false;
    this.markerKind = undefined;
    this.markerEscape = false;
    this.activePromptRendered = false;
  }

  markPromptComplete() {
    this.activePromptRendered = false;
  }

  transform(bytes: Uint8Array, enabled: boolean): Uint8Array {
    if (!enabled) {
      this.reset();
      return bytes;
    }

    const output: number[] = [];
    let changed = false;
    for (const byte of bytes) {
      output.push(byte);
      if (this.insideMarker) {
        if (byte === 0x07 || (this.markerEscape && byte === 0x5c)) {
          if (this.markerKind === PROMPT_START_MARKER) {
            if (this.activePromptRendered) {
              output.push(...RESTORE_AND_CLEAR_CURSOR);
            }
            output.push(...SAVE_CURSOR);
            changed = true;
          } else if (this.markerKind === PROMPT_END_MARKER) {
            output.push(0x0d, 0x0a);
            this.activePromptRendered = true;
            changed = true;
          } else if (
            this.markerKind === COMMAND_START_MARKER
            || this.markerKind === COMMAND_END_MARKER
          ) {
            this.activePromptRendered = false;
          }
          this.insideMarker = false;
          this.markerKind = undefined;
          this.markerEscape = false;
        } else {
          this.markerEscape = byte === 0x1b;
        }
        continue;
      }

      if (this.awaitingMarkerKind) {
        this.awaitingMarkerKind = false;
        this.insideMarker = true;
        this.markerKind = byte;
        this.markerEscape = false;
        continue;
      }

      if (byte === 0x0a) {
        // A line break received from the PTY ends the currently editable prompt. This also covers
        // commands and interrupts entered on the Mac, whose input does not pass through onData.
        this.activePromptRendered = false;
      }

      if (byte === SHELL_INTEGRATION_OSC_PREFIX[this.prefixIndex]) {
        this.prefixIndex += 1;
        if (this.prefixIndex === SHELL_INTEGRATION_OSC_PREFIX.length) {
          this.prefixIndex = 0;
          this.awaitingMarkerKind = true;
        }
      } else {
        this.prefixIndex = byte === SHELL_INTEGRATION_OSC_PREFIX[0] ? 1 : 0;
      }
    }
    return changed ? Uint8Array.from(output) : bytes;
  }
}

export const TerminalSurface = forwardRef<TerminalSurfaceHandle, Props>(function TerminalSurface(
  { snapshot, bus, canResize, mirror, onViewport, onResize, onData, onFocus, onStreamGap },
  ref,
) {
  const container = useRef<HTMLDivElement>(null);
  const stage = useRef<HTMLDivElement | undefined>(undefined);
  const scrollIndicator = useRef<HTMLDivElement>(null);
  const scrollIndicatorTimer = useRef<number | undefined>(undefined);
  const terminal = useRef<Terminal | undefined>(undefined);
  const fit = useRef<FitAddon | undefined>(undefined);
  const mirrorDimensions = useRef(mirror);
  mirrorDimensions.current = mirror;
  const stream = useRef<string | undefined>(undefined);
  const sequence = useRef(0);
  const resizeCallback = useRef(onResize);
  resizeCallback.current = onResize;
  const viewportCallback = useRef(onViewport);
  viewportCallback.current = onViewport;
  const resizeEnabled = useRef(canResize);
  resizeEnabled.current = canResize;
  const lastReportedDimensions = useRef<string | undefined>(undefined);
  const lastReportedTarget = useRef<string | undefined>(undefined);
  const snapshotDimensions = useRef<string | undefined>(undefined);
  const snapshotUsesAlternateScreen = useRef(false);
  const lastClearedAlternateScreenDimensions = useRef<string | undefined>(undefined);
  const snapshotWriteGeneration = useRef(0);
  const snapshotWriteInFlight = useRef(false);
  const revealLatestSnapshot = useRef(false);
  const visualResizeSequence = useRef<number | undefined>(undefined);
  const visualResizeOverlay = useRef<HTMLDivElement | undefined>(undefined);
  const visualResizeQuietTimer = useRef<number | undefined>(undefined);
  const visualResizeDeadlineTimer = useRef<number | undefined>(undefined);
  const zeroWidthPrompt = useRef(false);
  const zeroWidthPromptTransformer = useRef(new ZeroWidthPromptTransformer());
  const dataCallback = useRef(onData);
  dataCallback.current = onData;
  const focusCallback = useRef(onFocus);
  focusCallback.current = onFocus;
  const streamGapCallback = useRef(onStreamGap);
  streamGapCallback.current = onStreamGap;

  useImperativeHandle(ref, () => ({
    focus: () => terminal.current?.focus(),
    blur: () => terminal.current?.blur(),
    paste: (text) => terminal.current?.paste(text),
  }), []);

  const clearVisualResize = useCallback(() => {
    if (visualResizeQuietTimer.current !== undefined) {
      window.clearTimeout(visualResizeQuietTimer.current);
      visualResizeQuietTimer.current = undefined;
    }
    if (visualResizeDeadlineTimer.current !== undefined) {
      window.clearTimeout(visualResizeDeadlineTimer.current);
      visualResizeDeadlineTimer.current = undefined;
    }
    visualResizeOverlay.current?.remove();
    visualResizeOverlay.current = undefined;
    visualResizeSequence.current = undefined;
  }, []);

  const beginVisualResize = useCallback((preserveCurrentFrame: boolean) => {
    clearVisualResize();
    const owner = container.current?.querySelector<HTMLElement>(".xterm");
    if (!owner) return;
    const overlay = document.createElement("div");
    overlay.className = "terminal-resize-freeze";
    overlay.setAttribute("aria-hidden", "true");
    if (preserveCurrentFrame) {
      const rows = owner.querySelector<HTMLElement>(".xterm-rows");
      if (rows) overlay.append(rows.cloneNode(true));
    } else {
      const status = document.createElement("span");
      status.textContent = "Fitting session…";
      overlay.append(status);
    }
    owner.append(overlay);
    visualResizeOverlay.current = overlay;
    // Some programs do not repaint for SIGWINCH. Never leave a stale cover over a live terminal.
    visualResizeDeadlineTimer.current = window.setTimeout(clearVisualResize, 700);
  }, [clearVisualResize]);

  const revealVisualResizeAfterQuiet = useCallback(() => {
    if (!visualResizeOverlay.current) return;
    if (visualResizeQuietTimer.current !== undefined) {
      window.clearTimeout(visualResizeQuietTimer.current);
    }
    // xterm write callbacks run only after each repaint fragment is parsed. Reveal once the final
    // fragment has remained quiet long enough to paint as one complete Claude/Codex frame.
    visualResizeQuietTimer.current = window.setTimeout(clearVisualResize, 90);
  }, [clearVisualResize]);

  /**
   * Presents the Mac's grid at its own dimensions, shrunk to fit the phone.
   *
   * The column count is copied from the Mac rather than fitted locally, so absolute cursor
   * positioning always lands where the Mac put it. Everything that would make it readable —
   * scaling, and the reader's own pinch-zoom — happens after rendering, in CSS. xterm is on its
   * DOM renderer here (no canvas addon is loaded), so the scaled text stays real text and
   * survives pinch-zoom crisply instead of magnifying rasterized pixels.
   */
  const applyMirrorLayout = useCallback(() => {
    const element = container.current;
    const instance = terminal.current;
    const surface = stage.current;
    const dimensions = mirrorDimensions.current;
    if (!element || !instance || !surface || !dimensions) return;
    if (dimensions.columns < 1 || dimensions.rows < 1) return;
    // Measure at natural scale. Deriving the ratio from an already-scaled frame would compound
    // it on every layout pass and shrink the grid away to nothing.
    surface.style.transform = "";
    surface.style.width = "";
    surface.style.height = "";
    if (instance.cols !== dimensions.columns || instance.rows !== dimensions.rows) {
      instance.resize(dimensions.columns, dimensions.rows);
    }
    if (revealLatestSnapshot.current) {
      instance.scrollToBottom();
      revealLatestSnapshot.current = false;
    }
    const screen = surface.querySelector<HTMLElement>(".xterm-screen");
    const natural = screen?.getBoundingClientRect();
    if (!natural?.width || !natural.height) return;
    const style = window.getComputedStyle(element);
    const scale = mirrorScale(
      element.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight),
      natural.width,
    );
    if (scale === undefined) return;
    surface.style.width = `${natural.width}px`;
    surface.style.height = `${natural.height}px`;
    surface.style.transformOrigin = "0 0";
    surface.style.transform = `scale(${scale})`;
  }, []);

  /**
   * Positions the scroll indicator. iOS Safari honors neither `scrollbar-width` nor
   * `scrollbar-color`, and hides its own overlay scrollbars during touch scrolling, so a phone
   * scrolling through a long agent transcript otherwise has no cue at all about where it is.
   *
   * The indicator stays visible while scrolled back — that is exactly when "where am I" is a live
   * question — and fades out shortly after returning to the bottom.
   */
  const refreshScrollIndicator = useCallback((reveal: boolean) => {
    const indicator = scrollIndicator.current;
    const instance = terminal.current;
    if (!indicator || !instance) return;
    const buffer = instance.buffer.active;
    const geometry = scrollIndicatorGeometry(instance.rows, buffer.baseY, buffer.viewportY);
    if (!geometry) {
      indicator.style.opacity = "0";
      return;
    }
    indicator.style.height = `${geometry.heightPercent}%`;
    indicator.style.top = `${geometry.topPercent}%`;
    if (!reveal && indicator.style.opacity === "0") return;
    indicator.style.opacity = "1";
    if (scrollIndicatorTimer.current !== undefined) {
      window.clearTimeout(scrollIndicatorTimer.current);
      scrollIndicatorTimer.current = undefined;
    }
    if (!geometry.atBottom) return;
    scrollIndicatorTimer.current = window.setTimeout(() => {
      if (scrollIndicator.current) scrollIndicator.current.style.opacity = "0";
    }, 1_200);
  }, []);

  const fitAndReport = useCallback(() => {
    const element = container.current;
    const instance = terminal.current;
    const fitAddon = fit.current;
    if (!element || !instance || !fitAddon) return;
    if (snapshotWriteInFlight.current) return;
    if (mirrorDimensions.current) {
      applyMirrorLayout();
      return;
    }
    const bounds = element.getBoundingClientRect();
    if (bounds.width < 160 || bounds.height < 72) return;
    const proposed = fitAddon.proposeDimensions();
    if (
      proposed
      && resizeEnabled.current
      && snapshotUsesAlternateScreen.current
      && snapshotDimensions.current !== `${proposed.cols}:${proposed.rows}`
      && lastClearedAlternateScreenDimensions.current !== `${proposed.cols}:${proposed.rows}`
    ) {
      // A full-screen CLI repaints across several PTY reads after SIGWINCH. Preserve an already
      // settled phone frame (or show a neutral first-fit state) until that repaint is complete.
      beginVisualResize(lastClearedAlternateScreenDimensions.current !== undefined);
    }
    fitAddon.fit();
    if (revealLatestSnapshot.current) {
      // A desktop-sized primary-screen snapshot can retain a scrollback viewport above its live
      // prompt after it reflows into the shorter phone grid. Always reveal the newest snapshot
      // once, without forcing someone who intentionally scrolled up back to the bottom later.
      instance.scrollToBottom();
      revealLatestSnapshot.current = false;
    }
    if (isSafeTerminalResize(instance.cols, instance.rows, bounds.width, bounds.height)) {
      // Report the fitted phone viewport before this device owns the writer lease. The app uses
      // it to prepare the lease/resize handoff, but does not resize the Mac until control is held.
      viewportCallback.current(instance.cols, instance.rows);
    }
    if (
      !resizeEnabled.current
      || !isSafeTerminalResize(instance.cols, instance.rows, bounds.width, bounds.height)
    ) return;
    const dimensions = `${instance.cols}:${instance.rows}`;
    if (lastReportedDimensions.current === dimensions) return;
    if (
      snapshotUsesAlternateScreen.current
      && snapshotDimensions.current !== dimensions
      && lastClearedAlternateScreenDimensions.current !== dimensions
    ) {
      // A full-screen CLI redraws after SIGWINCH. Clear the resized local alternate buffer first
      // so cropped cells from the desktop-sized snapshot cannot survive that redraw on mobile.
      lastClearedAlternateScreenDimensions.current = dimensions;
      instance.write("\x1b[2J\x1b[H");
    }
    lastReportedDimensions.current = dimensions;
    if (visualResizeOverlay.current) visualResizeSequence.current = sequence.current;
    resizeCallback.current(instance.cols, instance.rows);
  }, [applyMirrorLayout, beginVisualResize]);

  const scheduleFitAndReport = useCallback(() => {
    // The double rAF defers fitting until after the pending React commit has laid out — but
    // rAF never fires in hidden or suspended tabs (a backgrounded phone PWA returning to the
    // foreground, a covered browser tab), which silently stalls the lease/resize handshake.
    // Race a timer that provides the same post-commit scheduling without depending on paint.
    let completed = false;
    const run = () => {
      if (completed) return;
      completed = true;
      window.clearTimeout(timer);
      fitAndReport();
    };
    const timer = window.setTimeout(run, 60);
    requestAnimationFrame(() => requestAnimationFrame(run));
  }, [fitAndReport]);

  const drainBufferedFrames = useCallback(function drain(
    streamId: string,
    writeGeneration: number,
  ) {
    const instance = terminal.current;
    if (
      !instance
      || stream.current !== streamId
      || snapshotWriteGeneration.current !== writeGeneration
    ) return;
    const frames = bus.framesAfter(streamId, sequence.current);
    if (!frames.length) {
      snapshotWriteInFlight.current = false;
      scheduleFitAndReport();
      return;
    }
    if (frames[0]?.sequence !== sequence.current + 1) {
      snapshotWriteInFlight.current = false;
      streamGapCallback.current?.();
      return;
    }
    const byteLength = frames.reduce((total, frame) => total + frame.payload.byteLength, 0);
    const payload = new Uint8Array(byteLength);
    let offset = 0;
    for (const frame of frames) {
      if (frame.sequence !== sequence.current + 1) {
        snapshotWriteInFlight.current = false;
        streamGapCallback.current?.();
        return;
      }
      payload.set(frame.payload, offset);
      offset += frame.payload.byteLength;
      sequence.current = frame.sequence;
    }
    bus.discardThrough(streamId, sequence.current);
    instance.write(
      zeroWidthPromptTransformer.current.transform(payload, zeroWidthPrompt.current),
      () => drain(streamId, writeGeneration),
    );
  }, [bus, scheduleFitAndReport]);

  useEffect(() => {
    if (!container.current) return;
    const instance = new Terminal({
      allowProposedApi: false,
      cursorBlink: false,
      disableStdin: false,
      fontFamily: "'JetBrains Mono Variable', 'JetBrains Mono', ui-monospace, monospace",
      fontSize: window.innerWidth < 600 ? 12 : 13,
      lineHeight: 1.18,
      scrollback: 10_000,
      theme: {
        background: "#050712",
        foreground: "#eef0f6",
        cursor: "#bfff00",
        selectionBackground: "#bfff0038",
        black: "#0b0f1e",
        brightBlack: "#5c647d",
        red: "#ff6568",
        green: "#05df72",
        yellow: "#fac800",
        blue: "#7aa8ff",
        magenta: "#be8cff",
        cyan: "#68d6df",
        white: "#eef0f6",
      },
    });
    const fitAddon = new FitAddon();
    instance.loadAddon(fitAddon);
    instance.loadAddon(new WebLinksAddon((_event, uri) => window.open(uri, "_blank", "noopener,noreferrer")));
    // xterm sizes `.xterm` from its in-flow `.xterm-screen`, so an explicitly sized wrapper is
    // what lets mirror mode scale the grid as one box without disturbing the scroll viewport.
    const surface = document.createElement("div");
    surface.className = "terminal-stage";
    container.current.append(surface);
    stage.current = surface;
    instance.open(surface);
    const helperTextarea = container.current.querySelector<HTMLTextAreaElement>(".xterm-helper-textarea");
    if (helperTextarea) {
      helperTextarea.id = "remote-control-terminal-input";
      // This is a raw PTY input surface, never a profile, credential, payment, or address field.
      // Suppress browser writing aids where they are honored and avoid semantic names that can
      // trigger AutoFill heuristics. iOS may still show its system-owned AutoFill shortcut bar.
      helperTextarea.removeAttribute("name");
      helperTextarea.autocomplete = "off";
      helperTextarea.autocapitalize = "off";
      helperTextarea.setAttribute("autocorrect", "off");
      helperTextarea.spellcheck = false;
      helperTextarea.setAttribute("aria-autocomplete", "none");
    }
    terminal.current = instance;
    fit.current = fitAddon;

    let resizeTimer = 0;
    const observer = new ResizeObserver(() => {
      window.clearTimeout(resizeTimer);
      resizeTimer = window.setTimeout(() => {
        fitAndReport();
      }, 180);
    });
    observer.observe(container.current);
    const unsubscribe = bus.subscribe((frame) => {
      if (frame.streamId !== stream.current || frame.sequence <= sequence.current) return;
      // Frames can reach the WebSocket immediately after its JSON snapshot, before React commits
      // that snapshot. TerminalBus retains them, and the snapshot callback drains them as one
      // ordered byte stream so an ANSI sequence split across frames can never lose its prefix.
      if (snapshotWriteInFlight.current) return;
      if (frame.sequence !== sequence.current + 1) {
        streamGapCallback.current?.();
        return;
      }
      sequence.current = frame.sequence;
      bus.discardThrough(frame.streamId, frame.sequence);
      instance.write(
        zeroWidthPromptTransformer.current.transform(frame.payload, zeroWidthPrompt.current),
        () => {
          if (
            visualResizeSequence.current !== undefined
            && frame.sequence > visualResizeSequence.current
          ) revealVisualResizeAfterQuiet();
        },
      );
    });
    const dataSubscription = instance.onData((data) => {
      if (/[\r\n\x03\x04]/u.test(data)) {
        zeroWidthPromptTransformer.current.markPromptComplete();
      }
      dataCallback.current?.(data);
    });
    const focusListener = () => focusCallback.current?.();
    container.current.addEventListener("focusin", focusListener);
    const scrollSubscription = instance.onScroll(() => refreshScrollIndicator(true));
    // Streaming output changes how much scrollback exists without scrolling the viewport, so the
    // indicator's proportions are refreshed on render — without reviving a faded-out indicator.
    const renderSubscription = instance.onRender(() => refreshScrollIndicator(false));
    scheduleFitAndReport();
    return () => {
      window.clearTimeout(resizeTimer);
      if (scrollIndicatorTimer.current !== undefined) {
        window.clearTimeout(scrollIndicatorTimer.current);
        scrollIndicatorTimer.current = undefined;
      }
      unsubscribe();
      dataSubscription.dispose();
      scrollSubscription.dispose();
      renderSubscription.dispose();
      container.current?.removeEventListener("focusin", focusListener);
      observer.disconnect();
      clearVisualResize();
      instance.dispose();
      surface.remove();
      stage.current = undefined;
      terminal.current = undefined;
    };
  }, [
    bus,
    clearVisualResize,
    fitAndReport,
    refreshScrollIndicator,
    revealVisualResizeAfterQuiet,
    scheduleFitAndReport,
  ]);

  useEffect(() => {
    const instance = terminal.current;
    if (!instance || !snapshot) return;
    clearVisualResize();
    const snapshotTarget = [
      snapshot.target.app_instance_id,
      snapshot.target.project_id,
      snapshot.target.tab_id,
      snapshot.target.pane_id,
    ].join(":");
    if (lastReportedTarget.current !== snapshotTarget) {
      lastReportedDimensions.current = undefined;
      lastReportedTarget.current = snapshotTarget;
    }
    const snapshotBytes = base64ToBytes(snapshot.data_base64);
    zeroWidthPrompt.current = snapshot.zero_width_prompt;
    zeroWidthPromptTransformer.current.reset();
    snapshotDimensions.current = `${snapshot.dimensions.columns}:${snapshot.dimensions.rows}`;
    snapshotUsesAlternateScreen.current = snapshotBytes.length >= 8
      && snapshotBytes[0] === 0x1b
      && snapshotBytes[1] === 0x5b
      && snapshotBytes[2] === 0x3f
      && snapshotBytes[3] === 0x31
      && snapshotBytes[4] === 0x30
      && snapshotBytes[5] === 0x34
      && snapshotBytes[6] === 0x39
      && snapshotBytes[7] === 0x68;
    lastClearedAlternateScreenDimensions.current = undefined;
    stream.current = snapshot.stream_id;
    sequence.current = snapshot.terminal_sequence;
    const writeGeneration = snapshotWriteGeneration.current + 1;
    snapshotWriteGeneration.current = writeGeneration;
    snapshotWriteInFlight.current = true;
    revealLatestSnapshot.current = true;
    // Parse an authoritative snapshot at the dimensions of the native grid that produced it.
    // Writing a 320-column Codex/Claude alternate screen into xterm's 80-column default causes
    // hard wraps before the first mobile fit and leaves duplicated suffixes after the CLI redraws.
    if (snapshot.zero_width_prompt && !mirrorDimensions.current) {
      // This snapshot is a logical prompt + command assembled from Clinch's separate grids, not
      // a native full-screen framebuffer. Parse it at the browser's fitted width so an OSC marker
      // at the prompt boundary cannot turn the Mac's old hard wrap into a stale phone row.
      //
      // Mirroring is excluded because there is no width of this device's own to reflow into:
      // the Mac owns the width, and matching it is the whole reason its cursor positioning still
      // lands where it was meant to.
      fit.current?.fit();
    } else if (
      instance.cols !== snapshot.dimensions.columns
      || instance.rows !== snapshot.dimensions.rows
    ) {
      instance.resize(snapshot.dimensions.columns, snapshot.dimensions.rows);
    }
    instance.reset();
    instance.write(zeroWidthPromptTransformer.current.transform(
      snapshotBytes,
      snapshot.zero_width_prompt,
    ), () => {
      if (
        stream.current !== snapshot.stream_id
        || snapshotWriteGeneration.current !== writeGeneration
      ) return;
      drainBufferedFrames(snapshot.stream_id, writeGeneration);
    });
  }, [clearVisualResize, drainBufferedFrames, snapshot]);

  useEffect(() => {
    if (!canResize) {
      lastReportedDimensions.current = undefined;
      return;
    }
    scheduleFitAndReport();
  }, [canResize, scheduleFitAndReport]);

  // Keyed on the dimensions themselves, not the object: the Mac re-sends a fresh pane snapshot
  // every second, and re-running this on each identical one would restart the layout constantly.
  const mirrorKey = mirror ? `${mirror.columns}:${mirror.rows}` : undefined;
  useEffect(() => {
    const surface = stage.current;
    if (!surface) return;
    if (!mirrorKey) {
      // Leaving mirror mode hands the width back to this device, so drop the scaled presentation
      // and let the next fit choose a column count for this screen again.
      surface.style.transform = "";
      surface.style.width = "";
      surface.style.height = "";
      lastReportedDimensions.current = undefined;
    }
    scheduleFitAndReport();
  }, [mirrorKey, scheduleFitAndReport]);

  return (
    <div className="terminal-surface" ref={container} aria-label="Selected Clinch terminal output">
      <div
        className="terminal-scroll-indicator"
        ref={scrollIndicator}
        role="presentation"
        aria-hidden="true"
      />
    </div>
  );
});
