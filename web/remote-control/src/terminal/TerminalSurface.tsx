import { useEffect, useRef } from "react";
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
  onResize: (columns: number, rows: number) => void;
  onData?: (data: string) => void;
  onFocus?: () => void;
}

export function TerminalSurface({ snapshot, bus, canResize, onResize, onData, onFocus }: Props) {
  const container = useRef<HTMLDivElement>(null);
  const terminal = useRef<Terminal | undefined>(undefined);
  const fit = useRef<FitAddon | undefined>(undefined);
  const stream = useRef<string | undefined>(undefined);
  const sequence = useRef(0);
  const resizeCallback = useRef(onResize);
  resizeCallback.current = onResize;
  const resizeEnabled = useRef(canResize);
  resizeEnabled.current = canResize;
  const dataCallback = useRef(onData);
  dataCallback.current = onData;
  const focusCallback = useRef(onFocus);
  focusCallback.current = onFocus;

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
      fitAddon.fit();
      window.clearTimeout(resizeTimer);
      resizeTimer = window.setTimeout(() => {
        if (resizeEnabled.current) resizeCallback.current(instance.cols, instance.rows);
      }, 180);
    });
    observer.observe(container.current);
    const unsubscribe = bus.subscribe((frame) => {
      if (frame.streamId !== stream.current || frame.sequence <= sequence.current) return;
      sequence.current = frame.sequence;
      instance.write(frame.payload);
    });
    const dataSubscription = instance.onData((data) => dataCallback.current?.(data));
    const focusListener = () => focusCallback.current?.();
    container.current.addEventListener("focusin", focusListener);
    requestAnimationFrame(() => fitAddon.fit());
    return () => {
      window.clearTimeout(resizeTimer);
      unsubscribe();
      dataSubscription.dispose();
      container.current?.removeEventListener("focusin", focusListener);
      observer.disconnect();
      instance.dispose();
      terminal.current = undefined;
    };
  }, [bus]);

  useEffect(() => {
    const instance = terminal.current;
    if (!instance || !snapshot) return;
    stream.current = snapshot.stream_id;
    sequence.current = snapshot.terminal_sequence;
    instance.reset();
    instance.write(base64ToBytes(snapshot.data_base64));
    requestAnimationFrame(() => fit.current?.fit());
  }, [snapshot]);

  useEffect(() => {
    if (!canResize) return;
    requestAnimationFrame(() => {
      fit.current?.fit();
      const instance = terminal.current;
      if (instance) resizeCallback.current(instance.cols, instance.rows);
    });
  }, [canResize]);

  return <div className="terminal-surface" ref={container} aria-label="Selected Clinch terminal output" />;
}
