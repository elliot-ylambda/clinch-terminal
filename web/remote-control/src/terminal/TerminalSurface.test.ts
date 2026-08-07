import { describe, expect, it } from "vitest";

import {
  mirrorScale,
  scrollIndicatorGeometry,
  ZeroWidthPromptTransformer,
} from "./TerminalSurface";

describe("mirroring the Mac's grid onto a phone", () => {
  it("shrinks a wide Mac pane until its full width fits", () => {
    // A 250-column pane on a phone: unreadable at a glance, but laid out exactly as the Mac laid
    // it out, so pinch-zooming reveals real text instead of clamped, piled-up characters.
    expect(mirrorScale(380, 1800)).toBeCloseTo(0.2111, 4);
  });

  it("never magnifies a Mac pane narrower than the phone", () => {
    expect(mirrorScale(380, 200)).toBe(1);
  });

  it("reports no scale before the surface has been laid out", () => {
    expect(mirrorScale(0, 1800)).toBeUndefined();
    expect(mirrorScale(380, 0)).toBeUndefined();
  });
});

describe("the phone's scroll position indicator", () => {
  it("stays silent until something has scrolled off the top", () => {
    expect(scrollIndicatorGeometry(40, 0, 0)).toBeUndefined();
  });

  it("sizes the thumb by how much of the transcript is on screen", () => {
    const geometry = scrollIndicatorGeometry(40, 60, 60);
    expect(geometry?.heightPercent).toBeCloseTo(40, 4);
    expect(geometry?.topPercent).toBeCloseTo(60, 4);
    expect(geometry?.atBottom).toBe(true);
  });

  it("rides to the top of the track when scrolled all the way back", () => {
    const geometry = scrollIndicatorGeometry(40, 60, 0);
    expect(geometry?.topPercent).toBe(0);
    expect(geometry?.atBottom).toBe(false);
  });

  it("keeps a visible thumb in a very long transcript", () => {
    // 10k lines of scrollback would otherwise leave a sub-pixel sliver of nothing to see.
    const geometry = scrollIndicatorGeometry(40, 10_000, 5_000);
    expect(geometry?.heightPercent).toBe(5);
    expect(geometry?.topPercent).toBeCloseTo(47.5, 4);
  });
});

const encode = (value: string) => new TextEncoder().encode(value);
const decode = (value: Uint8Array) => new TextDecoder().decode(value);

describe("zero-width Clinch prompt translation", () => {
  it("stacks the zero-width command grid below its visible prompt", () => {
    const transformer = new ZeroWidthPromptTransformer();
    const prompt = "\x1b]133;A\x07➜ project \x1b]133;B\x07slowtest";
    expect(decode(transformer.transform(encode(prompt), true)))
      .toBe("\x1b]133;A\x07\x1b[s➜ project \x1b]133;B\x07\r\nslowtest");
  });

  it("recognizes an end-prompt marker split across terminal frames", () => {
    const transformer = new ZeroWidthPromptTransformer();
    const first = transformer.transform(encode("\x1b]133;A\x07➜ project \x1b]13"), true);
    const second = transformer.transform(encode("3;B\x07typed"), true);
    expect(decode(first) + decode(second))
      .toBe("\x1b]133;A\x07\x1b[s➜ project \x1b]133;B\x07\r\ntyped");
  });

  it("redraws an active prompt from its saved origin instead of duplicating it", () => {
    const transformer = new ZeroWidthPromptTransformer();
    transformer.transform(encode("\x1b]133;A\x07➜ project \x1b]133;B\x07"), true);

    const redraw = "\r\x1b[J\x1b]133;A\x07➜ project \x1b]133;B\x07typed";
    expect(decode(transformer.transform(encode(redraw), true))).toBe(
      "\r\x1b[J\x1b]133;A\x07\x1b[u\x1b[J\x1b[s➜ project \x1b]133;B\x07\r\ntyped",
    );
  });

  it("starts a new saved prompt after a command is submitted", () => {
    const transformer = new ZeroWidthPromptTransformer();
    transformer.transform(encode("\x1b]133;A\x07➜ project \x1b]133;B\x07typed"), true);
    transformer.markPromptComplete();

    const nextPrompt = "\r\n\x1b]133;A\x07➜ project \x1b]133;B\x07";
    expect(decode(transformer.transform(encode(nextPrompt), true))).toBe(
      "\r\n\x1b]133;A\x07\x1b[s➜ project \x1b]133;B\x07\r\n",
    );
  });

  it("leaves ordinary terminal and PS1 bytes untouched", () => {
    const transformer = new ZeroWidthPromptTransformer();
    const bytes = encode("command\r\noutput\rprogress");
    expect(transformer.transform(bytes, false)).toBe(bytes);
  });
});
