import { describe, expect, it } from "vitest";

import { base64ToBytes, bytesToBase64 } from "./storage";

describe("credential-safe byte encoding", () => {
  it("round trips arbitrary bytes", () => {
    const bytes = new Uint8Array([0, 1, 127, 128, 254, 255]);
    expect(base64ToBytes(bytesToBase64(bytes))).toEqual(bytes);
  });
});
