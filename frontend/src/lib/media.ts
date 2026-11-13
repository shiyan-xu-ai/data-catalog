import type { ByteRange } from "./hevc-player";

/** Largest byte range fetched into memory to play or save a clip of a container file. */
export const MAX_CLIP_BYTES = 512 << 20;

const MIME: Record<string, string> = { mp4: "video/mp4", m4v: "video/mp4", mov: "video/quicktime", webm: "video/webm" };

function extension(url: string): string {
  return /\.([a-z0-9]+)(?:[?#].*)?$/i.exec(url)?.[1].toLowerCase() ?? "";
}

/** The bytes `range` of the object at `url`, as a blob typed by the URL's extension. Fails when
 *  they exceed `MAX_CLIP_BYTES` (checked from the response size for an open-ended range). */
export async function fetchClip(url: string, range: ByteRange, signal?: AbortSignal): Promise<Blob> {
  const end = range.length != null ? range.start + range.length - 1 : "";
  const response = await fetch(url, { headers: { Range: `bytes=${range.start}-${end}` }, signal });
  if (response.status !== 206) {
    throw new Error(
      response.ok ? "The server does not serve byte ranges, so the clip cannot be cut out." : `HTTP ${response.status} reading the clip`,
    );
  }
  if (Number(response.headers.get("content-length")) > MAX_CLIP_BYTES) {
    void response.body?.cancel();
    throw new Error("The clip is too large to load in the browser.");
  }
  const blob = await response.blob();
  return blob.slice(0, range.length ?? blob.size, MIME[extension(url)]);
}

/** A file name for the clip: the object's name with the byte range before the extension. */
export function clipFileName(url: string, range: ByteRange): string {
  const name = decodeURIComponent(url.split(/[?#]/)[0].split("/").pop() ?? "clip");
  const dot = name.lastIndexOf(".");
  const [stem, ext] = dot > 0 ? [name.slice(0, dot), name.slice(dot)] : [name, ""];
  const end = range.length != null ? range.start + range.length : "";
  return `${stem}_${range.start}-${end}${ext}`;
}
