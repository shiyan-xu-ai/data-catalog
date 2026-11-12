import { AccessUnitAssembler, AnnexBSplitter, hevcCodec, nalType, NAL_SPS, type AccessUnit } from "./hevc";

export type PlayerStatus = "loading" | "playing" | "paused" | "ended" | "unsupported" | "error";

export interface PlayerState {
  status: PlayerStatus;
  message?: string;
  codec?: string;
  width?: number;
  height?: number;
  /** Frames shown so far. */
  frames: number;
  bytesRead: number;
  /** Object size, once a ranged response reports it. */
  bytesTotal?: number;
}

/** Bytes per ranged request. */
const CHUNK_BYTES = 2 << 20;
/** Decoded frames held ahead of the playhead; reading pauses beyond this. */
const MAX_BUFFERED_FRAMES = 30;
const MAX_DECODE_QUEUE = 30;

interface QueuedFrame {
  frame: VideoFrame;
  /** Presentation index; frames leave the decoder in display order. */
  index: number;
}

function sleep(ms: number): Promise<void> {
  const { promise, resolve } = Promise.withResolvers<void>();
  setTimeout(resolve, ms);
  return promise;
}

/** Plays a raw H.265 Annex-B stream from `url` on `canvas`: reads it in ranged chunks, splits it
 *  into pictures, decodes them with WebCodecs, and draws frames at `fps` (raw streams carry no
 *  timestamps). Reading pauses while enough frames are buffered or playback is paused. Reports
 *  progress through `onState`; `close` stops everything and releases frames and the decoder. */
export class HevcStreamPlayer {
  private readonly abort = new AbortController();
  private readonly splitter = new AnnexBSplitter();
  private readonly assembler = new AccessUnitAssembler();
  private decoder: VideoDecoder | null = null;
  private frames: QueuedFrame[] = [];
  private state: PlayerState = { status: "loading", frames: 0, bytesRead: 0 };
  private fps: number;
  private decoded = 0;
  private sawKey = false;
  private readDone = false;
  private clockStart: number | null = null;
  private pausedAt: number | null = null;
  private raf = 0;

  constructor(
    private readonly url: string,
    private readonly canvas: HTMLCanvasElement,
    fps: number,
    private readonly onState: (state: PlayerState) => void,
  ) {
    this.fps = fps;
  }

  start(): void {
    if (typeof VideoDecoder === "undefined") {
      this.update({ status: "unsupported", message: "This browser has no WebCodecs video decoder." });
      return;
    }
    this.update({ status: "loading" });
    this.raf = requestAnimationFrame(this.render);
    void this.read().catch((e: unknown) => {
      if (!this.abort.signal.aborted) this.update({ status: "error", message: String(e) });
    });
  }

  pause(): void {
    if (this.state.status !== "playing" && this.state.status !== "loading") return;
    this.pausedAt = performance.now();
    this.update({ status: "paused" });
  }

  resume(): void {
    if (this.state.status !== "paused") return;
    if (this.clockStart != null && this.pausedAt != null) this.clockStart += performance.now() - this.pausedAt;
    this.pausedAt = null;
    this.update({ status: this.clockStart == null ? "loading" : "playing" });
  }

  /** Changes the frame rate, keeping the next frame due now. */
  setFps(fps: number): void {
    const next = this.frames[0]?.index;
    if (this.clockStart != null && next != null) this.clockStart = performance.now() - (next * 1000) / fps;
    this.fps = fps;
  }

  close(): void {
    this.abort.abort();
    cancelAnimationFrame(this.raf);
    for (const { frame } of this.frames) frame.close();
    this.frames = [];
    if (this.decoder && this.decoder.state !== "closed") this.decoder.close();
  }

  private update(patch: Partial<PlayerState>): void {
    this.state = { ...this.state, ...patch };
    this.onState(this.state);
    // Nothing more can play: stop any in-flight read instead of downloading the rest.
    if (patch.status === "unsupported" || patch.status === "error") this.abort.abort();
  }

  private get stopped(): boolean {
    return this.abort.signal.aborted;
  }

  /** Reads the object range by range; falls back to one streamed body when ranges are ignored. */
  private async read(): Promise<void> {
    let offset = 0;
    while (!this.stopped) {
      const response = await fetch(this.url, {
        headers: { Range: `bytes=${offset}-${offset + CHUNK_BYTES - 1}` },
        signal: this.abort.signal,
      });
      if (!response.ok) throw new Error(`HTTP ${response.status} reading the video`);
      const total = Number(response.headers.get("content-range")?.split("/")[1]);
      if (Number.isFinite(total)) this.update({ bytesTotal: total });
      const reader = response.body!.getReader();
      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        offset += value.length;
        this.update({ bytesRead: offset });
        await this.feed(this.splitter.push(value));
        if (this.stopped) return;
      }
      if (response.status !== 206 || !Number.isFinite(total) || offset >= total) break;
    }
    if (this.stopped) return;
    await this.feed(this.splitter.flush());
    const last = this.assembler.flush();
    if (last) await this.decode(last);
    await this.decoder?.flush();
    this.readDone = true;
  }

  private async feed(nals: Uint8Array[]): Promise<void> {
    for (const nal of nals) {
      if (this.stopped) return;
      if (!this.decoder && nalType(nal) === NAL_SPS) await this.configure(nal);
      const unit = this.assembler.push(nal);
      if (unit) await this.decode(unit);
    }
  }

  private async configure(sps: Uint8Array): Promise<void> {
    const codec = hevcCodec(sps);
    this.update({ codec });
    const { supported } = await VideoDecoder.isConfigSupported({ codec });
    if (!supported) {
      this.update({ status: "unsupported", message: `This browser cannot decode H.265 (${codec}).` });
      return;
    }
    this.decoder = new VideoDecoder({
      output: (frame) => {
        this.frames.push({ frame, index: this.decoded++ });
        if (this.state.width !== frame.displayWidth) this.update({ width: frame.displayWidth, height: frame.displayHeight });
      },
      error: (e) => this.update({ status: "error", message: e.message }),
    });
    this.decoder.configure({ codec });
  }

  private async decode(unit: AccessUnit): Promise<void> {
    const decoder = this.decoder;
    if (!decoder || decoder.state !== "configured") return;
    // Decoding must start at a random access point.
    if (!this.sawKey && !unit.key) return;
    this.sawKey = true;
    while (!this.stopped && (this.frames.length >= MAX_BUFFERED_FRAMES || decoder.decodeQueueSize >= MAX_DECODE_QUEUE)) {
      await sleep(20);
    }
    if (this.stopped) return;
    decoder.decode(
      new EncodedVideoChunk({ type: unit.key ? "key" : "delta", timestamp: 0, data: unit.data }),
    );
  }

  private readonly render = (now: number): void => {
    if (this.abort.signal.aborted) return;
    this.raf = requestAnimationFrame(this.render);
    if (this.state.status === "paused") return;
    const head = this.frames[0];
    if (!head) {
      if (this.readDone && this.state.status === "playing") this.update({ status: "ended" });
      return;
    }
    if (this.clockStart == null) {
      this.clockStart = now - (head.index * 1000) / this.fps;
      this.update({ status: "playing" });
    }
    // Show the latest due frame and drop any older ones that are also due.
    let shown: QueuedFrame | null = null;
    while (this.frames.length > 0 && now >= this.clockStart + (this.frames[0].index * 1000) / this.fps) {
      shown?.frame.close();
      shown = this.frames.shift()!;
    }
    if (!shown) return;
    const { frame } = shown;
    if (this.canvas.width !== frame.displayWidth) this.canvas.width = frame.displayWidth;
    if (this.canvas.height !== frame.displayHeight) this.canvas.height = frame.displayHeight;
    this.canvas.getContext("2d")?.drawImage(frame, 0, 0);
    frame.close();
    this.update({ frames: shown.index + 1 });
  };
}
