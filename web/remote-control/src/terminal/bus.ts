import type { TerminalOutputFrame } from "../protocol/binary";

type Listener = (frame: TerminalOutputFrame) => void;

const MAX_BUFFERED_FRAMES_PER_STREAM = 256;

export class TerminalBus {
  private listeners = new Set<Listener>();
  private buffered = new Map<string, TerminalOutputFrame[]>();
  private latestSequence = new Map<string, number>();

  emit(frame: TerminalOutputFrame): void {
    this.latestSequence.set(frame.streamId, frame.sequence);
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

  sequenceFor(streamId: string): number {
    return this.latestSequence.get(streamId) ?? 0;
  }

  /**
   * Waits until output newer than `sequence` has arrived and then gone quiet briefly. A PTY resize
   * is acknowledged before an interactive shell has necessarily handled SIGWINCH; this barrier
   * lets callers keep keyboard input behind the shell's asynchronous repaint. The bounded fallback
   * covers programs that intentionally emit nothing when resized.
   */
  waitForQuiescenceAfter(
    streamId: string,
    sequence: number,
    quietMs = 60,
    timeoutMs = 300,
  ): Promise<void> {
    return new Promise((resolve) => {
      let quietTimer: ReturnType<typeof setTimeout> | undefined;
      let unsubscribe = () => {};
      const finish = () => {
        if (quietTimer) clearTimeout(quietTimer);
        clearTimeout(timeoutTimer);
        unsubscribe();
        resolve();
      };
      const sawActivity = () => {
        if (quietTimer) clearTimeout(quietTimer);
        quietTimer = setTimeout(finish, quietMs);
      };
      const timeoutTimer = setTimeout(finish, timeoutMs);
      unsubscribe = this.subscribe((frame) => {
        if (frame.streamId === streamId && frame.sequence > sequence) sawActivity();
      });
      if (this.sequenceFor(streamId) > sequence) sawActivity();
    });
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
