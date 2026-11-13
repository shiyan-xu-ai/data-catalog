import { useEffect, useRef, useState } from "react";
import { useActive } from "@/lib/active";
import type { ByteRange } from "@/lib/hevc-player";
import { MAX_CLIP_BYTES, fetchClip } from "@/lib/media";
import { Skeleton } from "@/components/ui/skeleton";

const note = (text: string) => <p className="text-xs text-muted-foreground">{text}</p>;

/** A `<video>` for a container file at `url`. With `range`, only those bytes are fetched into
 *  memory and played, since a browser cannot play a slice of a remote file; larger clips than
 *  `MAX_CLIP_BYTES` are not played. Pauses while hidden. */
export function ClipVideo({ url, range, onFailed }: { url: string; range?: ByteRange; onFailed: () => void }) {
  const video = useRef<HTMLVideoElement>(null);
  const visible = useActive();
  useEffect(() => {
    if (!visible) video.current?.pause();
  }, [visible]);

  const [clipUrl, setClipUrl] = useState<string>();
  const [error, setError] = useState<string>();
  const start = range?.start;
  const length = range?.length;
  const tooBig = length != null && length > MAX_CLIP_BYTES;
  useEffect(() => {
    if (start == null || length == null || tooBig) return;
    const controller = new AbortController();
    let made: string | undefined;
    fetchClip(url, { start, length }, controller.signal)
      .then((blob) => {
        made = URL.createObjectURL(blob);
        setClipUrl(made);
      })
      .catch((e: unknown) => {
        if (!controller.signal.aborted) setError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      controller.abort();
      if (made) URL.revokeObjectURL(made);
      setClipUrl(undefined);
      setError(undefined);
    };
  }, [url, start, length, tooBig]);

  if (tooBig) return note("This clip is too large to play in the browser.");
  if (error) return note(`Could not load the clip: ${error}`);
  if (range && !clipUrl) return <Skeleton className="aspect-video w-full" />;
  return (
    <video
      ref={video}
      controls
      preload="metadata"
      src={range ? clipUrl : url}
      onError={onFailed}
      className="aspect-video w-full rounded bg-black"
    />
  );
}
