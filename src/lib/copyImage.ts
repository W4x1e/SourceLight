/**
 * Copying a photo to the clipboard.
 *
 * The backend renders the photo with its edits into a PNG (the only image the
 * web clipboard takes) and the page puts that file on the clipboard. The item
 * is made at once, from a promise of the image, so the click or key press that
 * asked still counts as the gesture the clipboard needs — WebKit on a Mac
 * refuses a write that starts after an await.
 */

/** Whether the user has text selected, which Ctrl+C should copy instead. */
export function textIsSelected(): boolean {
  return (window.getSelection?.()?.toString() ?? "").length > 0;
}

/** A PNG's size, from its header: width and height are the first two fields
 *  of IHDR, big-endian, at bytes 16 and 20. Null for anything else. */
export function pngSize(bytes: Uint8Array): { width: number; height: number } | null {
  const sig = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
  if (bytes.length < 24 || sig.some((b, i) => bytes[i] !== b)) return null;
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  return { width: view.getUint32(16), height: view.getUint32(20) };
}
