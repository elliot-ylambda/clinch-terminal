import { describe, expect, it } from "vitest";

import { BINARY_FRAME_HEADER_BYTES, BINARY_FRAME_MAGIC, PROTOCOL_VERSION } from "../generated/constants";
import { decodeTerminalOutput, encodeUploadChunk } from "./binary";

const id = "12345678-1234-5678-9abc-def012345678";

describe("binary framing", () => {
  it("encodes an ordered upload chunk", () => {
    const payload = new Uint8Array([0, 1, 2, 255]);
    const encoded = new Uint8Array(encodeUploadChunk(id, 42, payload));
    const view = new DataView(encoded.buffer);
    expect([...encoded.slice(0, 2)]).toEqual([...BINARY_FRAME_MAGIC]);
    expect(view.getUint16(2)).toBe(PROTOCOL_VERSION);
    expect(encoded[4]).toBe(2);
    expect(view.getBigUint64(24)).toBe(42n);
    expect([...encoded.slice(BINARY_FRAME_HEADER_BYTES)]).toEqual([...payload]);
  });

  it("decodes terminal bytes and rejects the upload frame kind", () => {
    const upload = new Uint8Array(encodeUploadChunk(id, 7, new Uint8Array([65, 66])));
    upload[4] = 1;
    const decoded = decodeTerminalOutput(upload.buffer);
    expect(decoded.streamId).toBe(id);
    expect(decoded.sequence).toBe(7);
    expect([...decoded.payload]).toEqual([65, 66]);

    upload[4] = 2;
    expect(() => decodeTerminalOutput(upload.buffer)).toThrow("Unexpected binary frame kind");
  });

  it("rejects malformed identifiers before sending", () => {
    expect(() => encodeUploadChunk("../not-an-id", 0, new Uint8Array([1]))).toThrow("Invalid upload identifier");
  });
});
