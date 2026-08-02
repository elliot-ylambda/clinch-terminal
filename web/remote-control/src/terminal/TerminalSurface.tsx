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

  const fitAndReport = useCallback(() => {
    const element = container.current;
    const instance = terminal.current;
    const fitAddon = fit.current;
    if (!element || !instance || !fitAddon) return;
    if (snapshotWriteInFlight.current) return;
    const bounds = element.getBoundingClientRect();
    if (bounds.width < 160 || bounds.height < 72) return;
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
    resizeCallback.current(instance.cols, instance.rows);
  }, []);

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
    instance.write(payload, () => drain(streamId, writeGeneration));
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
      instance.write(frame.payload);
    });
    const dataSubscription = instance.onData((data) => dataCallback.current?.(data));
    const focusListener = () => focusCallback.current?.();
    container.current.addEventListener("focusin", focusListener);
    requestAnimationFrame(() => requestAnimationFrame(fitAndReport));
    return () => {
      window.clearTimeout(resizeTimer);
      unsubscribe();
      dataSubscription.dispose();
      container.current?.removeEventListener("focusin", focusListener);
      observer.disconnect();
      instance.dispose();
      terminal.current = undefined;
    };
  }, [bus, fitAndReport]);

  useEffect(() => {
    const instance = terminal.current;
    if (!instance || !snapshot) return;
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
    if (
      instance.cols !== snapshot.dimensions.columns
      || instance.rows !== snapshot.dimensions.rows
    ) {
      instance.resize(snapshot.dimensions.columns, snapshot.dimensions.rows);
    }
    instance.reset();
    instance.write(snapshotBytes, () => {
      if (
        stream.current !== snapshot.stream_id
        || snapshotWriteGeneration.current !== writeGeneration
      ) return;
      drainBufferedFrames(snapshot.stream_id, writeGeneration);
    });
  }, [drainBufferedFrames, snapshot]);

  useEffect(() => {
    if (!canResize) {
      lastReportedDimensions.current = undefined;
      return;
    }
    requestAnimationFrame(() => requestAnimationFrame(fitAndReport));
  }, [canResize, fitAndReport]);

  return <div className="terminal-surface" ref={container} aria-label="Selected Clinch terminal output" />;
});
