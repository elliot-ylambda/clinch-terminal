import { afterEach, describe, expect, it, vi } from "vitest";

import type { TerminalOutputFrame } from "../protocol/binary";
import { TerminalBus } from "./bus";

function frame(sequence: number, text: string): TerminalOutputFrame {
  return {
    streamId: "11111111-1111-4111-8111-111111111111",
    sequence,
    payload: new TextEncoder().encode(text),
  };
}

describe("terminal frame handoff", () => {
  afterEach(() => vi.useRealTimers());

  it("retains frames that arrive before React commits their snapshot", () => {
    const bus = new TerminalBus();
    bus.emit(frame(1, "\x1b["));
    bus.emit(frame(2, "mready"));

    expect(bus.framesAfter("11111111-1111-4111-8111-111111111111", 0))
      .toEqual([frame(1, "\x1b["), frame(2, "mready")]);

    bus.discardThrough("11111111-1111-4111-8111-111111111111", 2);
    expect(bus.framesAfter("11111111-1111-4111-8111-111111111111", 2)).toEqual([]);
    expect(bus.sequenceFor("11111111-1111-4111-8111-111111111111")).toBe(2);
  });

  it("waits for a post-resize frame to finish before releasing input", async () => {
    vi.useFakeTimers();
    const bus = new TerminalBus();
    let settled = false;
    const waiting = bus
      .waitForQuiescenceAfter("11111111-1111-4111-8111-111111111111", 0, 60, 300)
      .then(() => { settled = true; });

    await vi.advanceTimersByTimeAsync(100);
    expect(settled).toBe(false);
    bus.emit(frame(1, "\r\x1b[2Kprompt"));
    await vi.advanceTimersByTimeAsync(59);
    expect(settled).toBe(false);
    bus.emit(frame(2, " "));
    await vi.advanceTimersByTimeAsync(60);
    await waiting;
    expect(settled).toBe(true);
  });

  it("uses a bounded fallback when a resized program emits no output", async () => {
    vi.useFakeTimers();
    const bus = new TerminalBus();
    let settled = false;
    const waiting = bus
      .waitForQuiescenceAfter("11111111-1111-4111-8111-111111111111", 0, 60, 300)
      .then(() => { settled = true; });

    await vi.advanceTimersByTimeAsync(299);
    expect(settled).toBe(false);
    await vi.advanceTimersByTimeAsync(1);
    await waiting;
    expect(settled).toBe(true);
  });
});
