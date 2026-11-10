// Files sent with a message: read here, whole, and carried in the message.
//
// The host saves each one in the sandbox and tells the agent where, and a
// picture goes to the model as well (see `hura-agent.mjs`). So all this side
// does is turn what was dropped, pasted or picked into base64, and refuse
// early what the host would refuse late: a message whose files come to more
// than one WebSocket frame can carry.

import type { ChatUpload } from "../gen/ChatUpload";

/// Kept in step with `UPLOAD_BYTES` in `hura-agent.mjs`.
export const UPLOAD_BYTES = 10 << 20;

/// A file waiting in the composer, with its size for the limit and the chip.
export type Attached = ChatUpload & { size: number; key: string };

let next = 0;

export function read(file: File): Promise<Attached> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(reader.error ?? new Error(`Could not read ${file.name}`));
    reader.onload = () => {
      // A data URL, of which the part after the comma is the base64.
      const url = String(reader.result);
      resolve({
        // A pasted screenshot is named `image.png` by WebKit, or nothing at all.
        name: file.name || "pasted",
        mime: file.type || "application/octet-stream",
        data: url.slice(url.indexOf(",") + 1),
        size: file.size,
        key: `f${next++}`,
      });
    };
    reader.readAsDataURL(file);
  });
}

export function isPicture(mime: string): boolean {
  return mime.startsWith("image/");
}

export function size(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1 << 20) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1 << 20)).toFixed(1)} MB`;
}
