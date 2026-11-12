import { useEffect, useRef, useState } from "react";
import { Pause, Play } from "lucide-react";
import { formatBytes, formatCount } from "@/lib/format";
import { HevcStreamPlayer, type PlayerState } from "@/lib/hevc-player";
import { Button } from "@/components/ui/button";

const FRAME_RATES = [5, 10, 15, 20, 24, 30, 60];

/** Player for a raw H.265 stream at `url`, decoded in the browser. Nothing is fetched until Play;
 *  a frame-rate picker sets playback speed since raw streams carry no timing. */
export function HevcPlayer({ url }: { url: string }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const player = useRef<HevcStreamPlayer | null>(null);
  const [state, setState] = useState<PlayerState | null>(null);
  const [fps, setFps] = useState(30);

  useEffect(() => () => player.current?.close(), []);

  function toggle() {
    const current = player.current;
    if (!current || !state || state.status === "ended" || state.status === "error") {
      current?.close();
      if (!canvas.current) return;
      const next = new HevcStreamPlayer(url, canvas.current, fps, setState);
      player.current = next;
      next.start();
    } else if (state.status === "paused") {
      current.resume();
    } else {
      current.pause();
    }
  }

  const running = state?.status === "playing" || state?.status === "loading";
  const blocked = state?.status === "unsupported";
  return (
    <div className="space-y-1.5">
      <canvas ref={canvas} className="aspect-video w-full rounded bg-black object-contain" />
      {state?.message && <p className="text-xs text-muted-foreground">{state.message}</p>}
      <div className="flex flex-wrap items-center gap-2 text-[11px] text-muted-foreground tabular-nums">
        <Button size="sm" variant="outline" className="h-7" onClick={toggle} disabled={blocked}>
          {running ? <Pause /> : <Play />}
          {running ? "Pause" : state?.status === "paused" ? "Resume" : state?.status === "ended" ? "Replay" : "Play"}
        </Button>
        <select
          aria-label="Frame rate"
          value={fps}
          onChange={(e) => {
            const next = Number(e.target.value);
            setFps(next);
            player.current?.setFps(next);
          }}
          className="h-7 rounded-md border border-input bg-background px-1.5 text-xs"
        >
          {FRAME_RATES.map((r) => (
            <option key={r} value={r}>
              {r} fps
            </option>
          ))}
        </select>
        {state && (
          <>
            <span>{state.status === "loading" ? "Buffering…" : state.status}</span>
            <span>frame {formatCount(state.frames)}</span>
            {state.width != null && (
              <span>
                {state.width}×{state.height}
              </span>
            )}
            {state.codec && <span className="font-mono">{state.codec}</span>}
            <span>
              {formatBytes(state.bytesRead)}
              {state.bytesTotal != null && ` / ${formatBytes(state.bytesTotal)}`}
            </span>
          </>
        )}
      </div>
    </div>
  );
}
