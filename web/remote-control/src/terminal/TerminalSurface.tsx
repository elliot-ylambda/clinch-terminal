import { forwardRef, useCallback, useEffect, useImperativeHandle, useRef } from "react";
import { FitAddon } from "@xterm/addon-fit";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";

import type { TerminalSnapshot } from "../generated/types/TerminalSnapshot";
import { base64ToBytes } from "../protocol/storage";
import type { TerminalBus } from "./bus";

interface Props {
  snapshot?: TerminalSnapshot;
  bus: TerminalBus;
  canResize: boolean;
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
  { snapshot, bus, canResize, onViewport, onResize, onData, onFocus, onStreamGap },
  ref,
) {
  const container = useRef<HTMLDivElement>(null);
  const terminal = useRef<Terminal | undefined>(undefined);
  const fit = useRef<FitAddon | undefined>(undefined);
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

  const fitAndReport = useCallback(() => {
    const element = container.current;
    const instance = terminal.current;
    const fitAddon = fit.current;
    if (!element || !instance || !fitAddon) return;
    if (snapshotWriteInFlight.current) return;
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
  }, [beginVisualResize]);

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
      requestAnimationFrame(() => requestAnimationFrame(fitAndReport));
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
  }, [bus, fitAndReport]);

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
    instance.open(container.current);
    const helperTextarea = container.current.querySelector<HTMLTextAreaElement>(".xterm-helper-textarea");
    if (helperTextarea) {
      helperTextarea.id = "remote-control-terminal-input";
      helperTextarea.name = "remote-control-terminal-input";
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
    requestAnimationFrame(() => requestAnimationFrame(fitAndReport));
    return () => {
      window.clearTimeout(resizeTimer);
      unsubscribe();
      dataSubscription.dispose();
      container.current?.removeEventListener("focusin", focusListener);
      observer.disconnect();
      clearVisualResize();
      instance.dispose();
      terminal.current = undefined;
    };
  }, [bus, clearVisualResize, fitAndReport, revealVisualResizeAfterQuiet]);

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
    if (snapshot.zero_width_prompt) {
      // This snapshot is a logical prompt + command assembled from Clinch's separate grids, not
      // a native full-screen framebuffer. Parse it at the browser's fitted width so an OSC marker
      // at the prompt boundary cannot turn the Mac's old hard wrap into a stale phone row.
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
    requestAnimationFrame(() => requestAnimationFrame(fitAndReport));
  }, [canResize, fitAndReport]);

  return <div className="terminal-surface" ref={container} aria-label="Selected Clinch terminal output" />;
});
