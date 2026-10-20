import type { RemoteDesktopFramePatch } from "./remoteDesktopFrame";

const MIB = 1024 * 1024;
export const MAX_PENDING_DESKTOP_PATCHES = 128;

export function isFullDesktopFrame(patch: RemoteDesktopFramePatch): boolean {
  return (
    patch.x === 0 &&
    patch.y === 0 &&
    patch.width === patch.desktopWidth &&
    patch.height === patch.desktopHeight
  );
}

/** Holds only pending uploads, not an additional copy of the desktop. */
export class RemoteDesktopFrameQueue {
  private patches: RemoteDesktopFramePatch[] = [];
  private buffers = new Set<ArrayBufferLike>();
  bytes = 0;
  peakBytes = 0;
  peakCount = 0;

  push(patch: RemoteDesktopFramePatch): boolean {
    const last = this.patches[this.patches.length - 1];
    if (
      isFullDesktopFrame(patch) ||
      (last &&
        (last.desktopWidth !== patch.desktopWidth ||
          last.desktopHeight !== patch.desktopHeight))
    ) {
      this.clear();
    }
    const buffer = patch.payload.buffer;
    const bytes =
      this.bytes + (this.buffers.has(buffer) ? 0 : buffer.byteLength);
    const budget = Math.min(
      128 * MIB,
      Math.max(64 * MIB, patch.desktopWidth * patch.desktopHeight * 4 + 44),
    );
    if (this.patches.length >= MAX_PENDING_DESKTOP_PATCHES || bytes > budget) {
      this.clear();
      return false;
    }
    this.patches.push(patch);
    this.buffers.add(buffer);
    this.bytes = bytes;
    this.peakBytes = Math.max(this.peakBytes, bytes);
    this.peakCount = Math.max(this.peakCount, this.patches.length);
    return true;
  }

  take(): RemoteDesktopFramePatch[] {
    const patches = this.patches;
    this.clear();
    return patches;
  }

  clear(): void {
    this.patches = [];
    this.buffers.clear();
    this.bytes = 0;
  }
}
