import { BINARY_FRAME_HEADER_BYTES, BINARY_FRAME_MAGIC, MAX_UPLOAD_CHUNK_BYTES, PROTOCOL_VERSION } from "../generated/constants";

export interface TerminalOutputFrame {
  streamId: string;
  sequence: number;
  payload: Uint8Array;
}

export function decodeTerminalOutput(buffer: ArrayBuffer): TerminalOutputFrame {
  const bytes = new Uint8Array(buffer);
  if (bytes.length < BINARY_FRAME_HEADER_BYTES) throw new Error("Short terminal frame");
  if (bytes[0] !== BINARY_FRAME_MAGIC[0] || bytes[1] !== BINARY_FRAME_MAGIC[1]) throw new Error("Invalid frame magic");
  const view = new DataView(buffer);
  if (view.getUint16(2) !== PROTOCOL_VERSION) throw new Error("Unsupported frame version");
  if (bytes[4] !== 1) throw new Error("Unexpected binary frame kind");
  if (bytes[5] || bytes[6] || bytes[7]) throw new Error("Unsupported binary frame flags");
  const sequence = Number(view.getBigUint64(24));
  return {
    streamId: uuidFromBytes(bytes.slice(8, 24)),
    sequence,
    payload: bytes.slice(BINARY_FRAME_HEADER_BYTES),
  };
}

export function encodeUploadChunk(uploadId: string, index: number, payload: Uint8Array): ArrayBuffer {
  if (payload.length > MAX_UPLOAD_CHUNK_BYTES) throw new Error("Upload chunk is too large");
  const frame = new Uint8Array(BINARY_FRAME_HEADER_BYTES + payload.length);
  const view = new DataView(frame.buffer);
  frame.set(BINARY_FRAME_MAGIC, 0);
  view.setUint16(2, PROTOCOL_VERSION);
  frame[4] = 2;
  frame.set(uuidToBytes(uploadId), 8);
  view.setBigUint64(24, BigInt(index));
  frame.set(payload, BINARY_FRAME_HEADER_BYTES);
  return frame.buffer;
}

function uuidFromBytes(bytes: Uint8Array): string {
  const hex = [...bytes].map((byte) => byte.toString(16).padStart(2, "0")).join("");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

function uuidToBytes(uuid: string): Uint8Array {
  const hex = uuid.replaceAll("-", "");
  if (!/^[0-9a-f]{32}$/i.test(hex)) throw new Error("Invalid upload identifier");
  return Uint8Array.from(hex.match(/.{2}/g) ?? [], (part) => Number.parseInt(part, 16));
}
