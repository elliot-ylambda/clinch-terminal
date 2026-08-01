import { describe, expect, it } from "vitest";

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
  it("retains frames that arrive before React commits their snapshot", () => {
    const bus = new TerminalBus();
    bus.emit(frame(1, "\x1b["));
    bus.emit(frame(2, "mready"));

    expect(bus.framesAfter("11111111-1111-4111-8111-111111111111", 0))
      .toEqual([frame(1, "\x1b["), frame(2, "mready")]);

    bus.discardThrough("11111111-1111-4111-8111-111111111111", 2);
    expect(bus.framesAfter("11111111-1111-4111-8111-111111111111", 2)).toEqual([]);
  });
});
