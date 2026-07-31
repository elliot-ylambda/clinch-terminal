import type { TerminalOutputFrame } from "../protocol/binary";

type Listener = (frame: TerminalOutputFrame) => void;

export class TerminalBus {
  private listeners = new Set<Listener>();

  emit(frame: TerminalOutputFrame): void {
    for (const listener of this.listeners) listener(frame);
  }

  subscribe(listener: Listener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }
}
