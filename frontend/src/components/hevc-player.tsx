import { useEffect, useRef, useState, type KeyboardEvent, type PointerEvent } from "react";
import { Maximize, Minimize, Pause, Play, RotateCcw } from "lucide-react";
import { formatBytes } from "@/lib/format";
import { HevcStreamPlayer, type PlayerState } from "@/lib/hevc-player";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

const FRAME_RATES = [5, 10, 15, 20, 24, 30, 60];
/** Arrow keys skip this many seconds. */
const SKIP_SECONDS = 5;

function clock(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  const h = Math.floor(s / 3600);
  const mm = String(Math.floor((s % 3600) / 60)).padStart(h > 0 ? 2 : 1, "0");
  const ss = String(s % 60).padStart(2, "0");
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}

/** Player for a raw H.265 stream at `url`, decoded in the browser. Nothing is fetched until Play.
 *  A frame-rate picker sets playback speed since raw streams carry no timing; the progress bar and
 *  duration are by byte position, so times are estimates. Click the video or press Space to play
 *  or pause, arrow keys skip, F toggles full screen. */
export function HevcPlayer({ url }: { url: string }) {
  const container = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const bar = useRef<HTMLDivElement>(null);
  const player = useRef<HevcStreamPlayer | null>(null);
  const [state, setState] = useState<PlayerState | null>(null);
  const [fps, setFps] = useState(30);
  /** Fraction under the pointer while dragging the progress bar. */
  const [scrub, setScrub] = useState<number | null>(null);
  const [fullscreen, setFullscreen] = useState(false);

  useEffect(() => () => player.current?.close(), []);
  useEffect(() => {
    const onChange = () => setFullscreen(document.fullscreenElement === container.current);
    document.addEventListener("fullscreenchange", onChange);
    return () => document.removeEventListener("fullscreenchange", onChange);
  }, []);

  const current = player.current;
  const status = state?.status;
  const running = status === "playing" || status === "loading";
  const blocked = status === "unsupported";
  const seekable = !!current?.seekable && !blocked && status !== "error";
  const total = state?.bytesTotal;

  function toggle() {
    if (blocked) return;
    if (current && status === "ended" && current.seekable) {
      current.seek(0);
    } else if (!current || !state || status === "ended" || status === "error") {
      current?.close();
      if (!canvas.current) return;
      const next = new HevcStreamPlayer(url, canvas.current, fps, setState);
      player.current = next;
      next.start();
    } else if (status === "paused") {
      current.resume();
    } else {
      current.pause();
    }
  }

  function skip(seconds: number) {
    if (!seekable || !state?.frameCount) return;
    current!.seek((state.frame + seconds * fps) / state.frameCount);
  }

  function fractionAt(e: PointerEvent): number {
    const rect = bar.current!.getBoundingClientRect();
    return Math.min(Math.max((e.clientX - rect.left) / rect.width, 0), 1);
  }

  function toggleFullscreen() {
    if (document.fullscreenElement) void document.exitFullscreen();
    else void container.current?.requestFullscreen();
  }

  function onKeyDown(e: KeyboardEvent) {
    if (e.target instanceof HTMLSelectElement) return;
    const actions: Record<string, () => void> = {
      " ": toggle,
      k: toggle,
      ArrowLeft: () => skip(-SKIP_SECONDS),
      ArrowRight: () => skip(SKIP_SECONDS),
      f: toggleFullscreen,
    };
    const action = actions[e.key];
    if (!action) return;
    e.preventDefault();
    action();
  }

  const played = scrub ?? (total ? state!.position / total : 0);
  const buffered = total && state ? { left: state.bufferStart / total, width: (state.bytesRead - state.bufferStart) / total } : null;
  const duration = state?.frameCount != null ? state.frameCount / fps : null;
  const at = scrub != null && duration != null ? scrub * duration : (state?.frame ?? 0) / fps;

  return (
    <div className="space-y-1.5">
      <div
        ref={container}
        tabIndex={0}
        onKeyDown={onKeyDown}
        className="group/player relative overflow-hidden rounded bg-black focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
      >
        <canvas
          ref={canvas}
          onClick={toggle}
          className={cn("w-full cursor-pointer object-contain", fullscreen ? "h-screen" : "aspect-video")}
        />
        {!running && !blocked && status !== "error" && (
          <button
            type="button"
            aria-label={status === "ended" ? "Replay" : "Play"}
            onClick={() => {
              toggle();
              // This button unmounts once playing; keep keyboard shortcuts working.
              container.current?.focus({ preventScroll: true });
            }}
            className="absolute inset-0 m-auto flex size-14 items-center justify-center rounded-full bg-black/60 text-white hover:bg-black/80"
          >
            {status === "ended" ? <RotateCcw className="size-6" /> : <Play className="size-6 fill-current" />}
          </button>
        )}
        <div
          className={cn(
            "absolute inset-x-0 bottom-0 bg-gradient-to-t from-black/80 to-transparent px-3 pt-6 pb-2 text-white transition-opacity",
            running && scrub == null && "opacity-0 group-hover/player:opacity-100 group-focus-within/player:opacity-100",
          )}
        >
          <div
            ref={bar}
            role="slider"
            aria-label="Seek"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={Math.round(played * 100)}
            aria-disabled={!seekable}
            onPointerDown={(e) => {
              if (!seekable) return;
              e.currentTarget.setPointerCapture(e.pointerId);
              setScrub(fractionAt(e));
            }}
            onPointerMove={(e) => scrub != null && setScrub(fractionAt(e))}
            onPointerUp={(e) => {
              if (scrub == null) return;
              current?.seek(fractionAt(e));
              setScrub(null);
            }}
            onPointerCancel={() => setScrub(null)}
            className={cn("group/bar relative flex h-3 items-center", seekable ? "cursor-pointer" : "cursor-default")}
          >
            <div className="relative h-1 w-full rounded-full bg-white/20 transition-[height] group-hover/bar:h-1.5">
              {buffered && (
                <div
                  className="absolute inset-y-0 rounded-full bg-white/45"
                  style={{ left: `${buffered.left * 100}%`, width: `${buffered.width * 100}%` }}
                />
              )}
              <div className="absolute inset-y-0 left-0 rounded-full bg-white" style={{ width: `${played * 100}%` }} />
            </div>
            {seekable && (
              <div
                className="absolute size-3 -translate-x-1/2 rounded-full bg-white opacity-0 shadow transition-opacity group-hover/bar:opacity-100"
                style={{ left: `${played * 100}%` }}
              />
            )}
          </div>
          <div className="mt-1 flex items-center gap-1 text-xs tabular-nums">
            <Button
              size="icon"
              variant="ghost"
              className="size-7 text-white hover:bg-white/15 hover:text-white"
              aria-label={running ? "Pause" : status === "ended" ? "Replay" : "Play"}
              onClick={toggle}
              disabled={blocked}
            >
              {running ? <Pause className="fill-current" /> : status === "ended" ? <RotateCcw /> : <Play className="fill-current" />}
            </Button>
            <span className="px-1">
              {clock(at)}
              {duration != null && ` / ≈${clock(duration)}`}
            </span>
            {status === "loading" && <span className="text-white/70">Buffering…</span>}
            <select
              aria-label="Frame rate"
              value={fps}
              onChange={(e) => {
                const next = Number(e.target.value);
                setFps(next);
                current?.setFps(next);
              }}
              className="ml-auto h-7 rounded-md bg-transparent px-1 text-xs hover:bg-white/15 [&>option]:text-foreground"
            >
              {FRAME_RATES.map((r) => (
                <option key={r} value={r}>
                  {r} fps
                </option>
              ))}
            </select>
            <Button
              size="icon"
              variant="ghost"
              className="size-7 text-white hover:bg-white/15 hover:text-white"
              aria-label={fullscreen ? "Exit full screen" : "Full screen"}
              onClick={toggleFullscreen}
            >
              {fullscreen ? <Minimize /> : <Maximize />}
            </Button>
          </div>
        </div>
      </div>
      {state?.message && <p className="text-xs text-muted-foreground">{state.message}</p>}
      {state && (
        <p className="flex flex-wrap gap-x-3 text-[11px] text-muted-foreground tabular-nums">
          {state.width != null && (
            <span>
              {state.width}×{state.height}
            </span>
          )}
          {state.codec && <span className="font-mono">{state.codec}</span>}
          <span>frame {state.frame + 1}</span>
          {total != null && (
            <span>
              read {formatBytes(state.bytesRead - state.bufferStart)} of {formatBytes(total)}
            </span>
          )}
        </p>
      )}
    </div>
  );
}
