import type { TerminalOutputFrame } from "../protocol/binary";

type Listener = (frame: TerminalOutputFrame) => void;

const MAX_BUFFERED_FRAMES_PER_STREAM = 256;

export class TerminalBus {
  private listeners = new Set<Listener>();
  private buffered = new Map<string, TerminalOutputFrame[]>();

  emit(frame: TerminalOutputFrame): void {
    const frames = this.buffered.get(frame.streamId) ?? [];
    frames.push(frame);
    if (frames.length > MAX_BUFFERED_FRAMES_PER_STREAM) frames.shift();
    this.buffered.set(frame.streamId, frames);
    for (const listener of this.listeners) listener(frame);
  }

  subscribe(listener: Listener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  framesAfter(streamId: string, sequence: number): TerminalOutputFrame[] {
    return (this.buffered.get(streamId) ?? []).filter((frame) => frame.sequence > sequence);
  }

  discardThrough(streamId: string, sequence: number): void {
    const remaining = (this.buffered.get(streamId) ?? [])
      .filter((frame) => frame.sequence > sequence);
    if (remaining.length) this.buffered.set(streamId, remaining);
    else this.buffered.delete(streamId);
  }
}
