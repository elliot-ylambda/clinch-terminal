import { describe, expect, it } from "vitest";

import { ZeroWidthPromptTransformer } from "./TerminalSurface";

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
