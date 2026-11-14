import { AccessUnitAssembler, annexB, AnnexBSplitter, hevcCodec, nalType, NAL_SPS, type AccessUnit, type NalUnit } from "./hevc";

export type PlayerStatus = "loading" | "playing" | "paused" | "ended" | "unsupported" | "error";

export interface PlayerState {
  status: PlayerStatus;
  message?: string;
  codec?: string;
  width?: number;
  height?: number;
  /** Index of the frame on screen; after a seek, estimated from its byte offset. */
  frame: number;
  /** Estimated frame count, from the object size and the average picture size read so far. */
  frameCount?: number;
  /** Stream offset of the frame on screen. */
  position: number;
  /** Bytes `[bufferStart, bytesRead)` have been read since playback started or last seeked. */
  bufferStart: number;
  bytesRead: number;
  /** Object size, once a ranged response reports it; seeking needs it. */
  bytesTotal?: number;
}

/** Bytes per ranged request. */
const CHUNK_BYTES = 2 << 20;
/** Decoded frames held ahead of the playhead; reading pauses beyond this. */
const MAX_BUFFERED_FRAMES = 30;
const MAX_DECODE_QUEUE = 30;
/** NAL types of the parameter sets a decoder needs before its first picture: VPS, SPS, PPS. */
const PARAMETER_SETS = [32, 33, 34];

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

/** A byte window of an object starting at `start`: `length` bytes, or to the object's end when
 *  `length` is absent. */
export interface ByteRange {
  start: number;
  length?: number;
}

/** The `Range` header for the chunk at `offset` within the window that starts at `origin` and is
 *  `limit` bytes long (unbounded for the whole object). */
export function rangeHeader(origin: number, offset: number, limit: number): string {
  return `bytes=${origin + offset}-${origin + Math.min(offset + CHUNK_BYTES, limit) - 1}`;
}

/** Plays a raw H.265 Annex-B stream from `url` on `canvas`: reads it in ranged chunks, splits it
 *  into pictures, decodes them with WebCodecs, and draws frames at `fps` (raw streams carry no
 *  timestamps). Reading pauses while enough frames are buffered. `seek` jumps to a byte fraction
 *  of the object and resumes at the next random access point there. Reports progress through
 *  `onState`; `close` stops everything and releases frames and the decoder. */
export class HevcStreamPlayer {
  private closed = false;
  /** Aborts the current read; replaced on every seek. */
  private session = new AbortController();
  private decoder: VideoDecoder | null = null;
  private codec = "";
  /** Latest VPS, SPS, and PPS by NAL type, for decoding from a picture that lacks them. */
  private readonly parameterSets = new Map<number, Uint8Array>();
  private frames: QueuedFrame[] = [];
  private state: PlayerState = { status: "loading", frame: 0, position: 0, bufferStart: 0, bytesRead: 0 };
  private fps: number;
  /** Index the decoder's next output frame gets. */
  private nextIndex = 0;
  /** Decoding (re)starts at a random access point... */
  private sawKey = false;
  /** ...without the RASL pictures that follow it, which reference pictures before it. */
  private skipRasl = false;
  private unitBytes = 0;
  private unitCount = 0;
  private readDone = false;
  private clockStart: number | null = null;
  private pausedAt: number | null = null;
  /** Draw the next buffered frame even while paused, to show where a seek landed. */
  private preview = false;
  private raf = 0;

  constructor(
    private readonly url: string,
    private readonly canvas: HTMLCanvasElement,
    fps: number,
    private readonly onState: (state: PlayerState) => void,
    /** The bytes of the object that hold the stream; the whole object when absent. All offsets
     *  the player reports are relative to `range.start`. */
    private readonly range?: ByteRange,
  ) {
    this.fps = fps;
    if (range?.length != null) this.state = { ...this.state, bytesTotal: range.length };
  }

  start(): void {
    if (typeof VideoDecoder === "undefined") {
      this.update({ status: "unsupported", message: "This browser has no WebCodecs video decoder." });
      return;
    }
    this.update({ status: "loading" });
    this.raf = requestAnimationFrame(this.render);
    this.read(0);
  }

  /** Whether `seek` can run: the object size is known and the decoder is configured. */
  get seekable(): boolean {
    return this.state.bytesTotal != null && this.decoder?.state === "configured";
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

  /** Jumps to `fraction` (0 to 1) of the object's bytes, keeping the paused state; from the end
   *  of the stream it plays again. */
  seek(fraction: number): void {
    const total = this.state.bytesTotal;
    const decoder = this.decoder;
    if (this.closed || total == null || !decoder || decoder.state !== "configured") return;
    const offset = Math.floor(Math.min(Math.max(fraction, 0), 1) * Math.max(total - 1, 0));
    this.session.abort();
    for (const { frame } of this.frames) frame.close();
    this.frames = [];
    decoder.reset();
    decoder.configure({ codec: this.codec });
    this.nextIndex = this.unitCount > 0 ? Math.round(offset / (this.unitBytes / this.unitCount)) : 0;
    this.clockStart = null;
    this.pausedAt = null;
    this.preview = true;
    const status = this.state.status === "paused" ? "paused" : "loading";
    this.update({ status, frame: this.nextIndex, position: offset, bufferStart: offset, bytesRead: offset });
    this.read(offset);
  }

  /** Changes the frame rate, keeping the next frame due now. */
  setFps(fps: number): void {
    const next = this.frames[0]?.index;
    if (this.clockStart != null && next != null) this.clockStart = performance.now() - (next * 1000) / fps;
    this.fps = fps;
  }

  close(): void {
    this.closed = true;
    this.session.abort();
    cancelAnimationFrame(this.raf);
    for (const { frame } of this.frames) frame.close();
    this.frames = [];
    if (this.decoder && this.decoder.state !== "closed") this.decoder.close();
  }

  private update(patch: Partial<PlayerState>): void {
    this.state = { ...this.state, ...patch };
    this.onState(this.state);
    // Nothing more can play: stop any in-flight read instead of downloading the rest.
    if (patch.status === "unsupported" || patch.status === "error") this.session.abort();
  }

  /** Starts a read session at `offset`, replacing any current one. */
  private read(offset: number): void {
    const session = new AbortController();
    this.session = session;
    this.readDone = false;
    this.sawKey = false;
    void this.readFrom(offset, session.signal).catch((e: unknown) => {
      if (!session.signal.aborted) this.update({ status: "error", message: String(e) });
    });
  }

  /** Reads the stream range by range from `start` (relative to the window); from 0 it falls back
   *  to one streamed body when ranges are ignored. */
  private async readFrom(start: number, signal: AbortSignal): Promise<void> {
    const splitter = new AnnexBSplitter(start);
    const assembler = new AccessUnitAssembler();
    const origin = this.range?.start ?? 0;
    const limit = this.range?.length ?? Infinity;
    let offset = start;
    while (!signal.aborted) {
      const response = await fetch(this.url, { headers: { Range: rangeHeader(origin, offset, limit) }, signal });
      if (!response.ok) throw new Error(`HTTP ${response.status} reading the video`);
      if ((start > 0 || origin > 0) && response.status !== 206) {
        throw new Error("The server does not serve byte ranges, so the video cannot seek.");
      }
      const objectSize = Number(response.headers.get("content-range")?.split("/")[1]);
      const total = Math.min(limit, objectSize - origin);
      if (Number.isFinite(total) && total !== this.state.bytesTotal) this.update({ bytesTotal: total });
      const reader = response.body!.getReader();
      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        const chunk = offset + value.length > limit ? value.subarray(0, limit - offset) : value;
        offset += chunk.length;
        this.update({ bytesRead: offset });
        await this.feed(splitter.push(chunk), assembler, signal);
        if (signal.aborted) return;
        if (offset >= limit) {
          void reader.cancel();
          break;
        }
      }
      if (response.status !== 206 || !Number.isFinite(total) || offset >= total) break;
    }
    if (signal.aborted) return;
    await this.feed(splitter.flush(), assembler, signal);
    const last = assembler.flush();
    if (last) await this.decode(last, signal);
    await this.decoder?.flush();
    if (!signal.aborted) this.readDone = true;
  }

  private async feed(nals: NalUnit[], assembler: AccessUnitAssembler, signal: AbortSignal): Promise<void> {
    for (const nal of nals) {
      if (signal.aborted) return;
      const type = nalType(nal.data);
      if (PARAMETER_SETS.includes(type)) this.parameterSets.set(type, nal.data);
      if (!this.decoder && type === NAL_SPS) await this.configure(nal.data);
      const unit = assembler.push(nal);
      if (unit) await this.decode(unit, signal);
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
    this.codec = codec;
    this.decoder = new VideoDecoder({
      output: (frame) => {
        this.frames.push({ frame, index: this.nextIndex++ });
        if (this.state.width !== frame.displayWidth) this.update({ width: frame.displayWidth, height: frame.displayHeight });
      },
      error: (e) => this.update({ status: "error", message: e.message }),
    });
    this.decoder.configure({ codec });
  }

  private async decode(unit: AccessUnit, signal: AbortSignal): Promise<void> {
    const decoder = this.decoder;
    if (!decoder || decoder.state !== "configured") return;
    let data = unit.data;
    if (!this.sawKey) {
      if (!unit.key) return;
      this.sawKey = true;
      this.skipRasl = true;
      if (!unit.hasSps) {
        const sets = annexB(PARAMETER_SETS.flatMap((t) => this.parameterSets.get(t) ?? []));
        data = new Uint8Array(sets.length + unit.data.length);
        data.set(sets);
        data.set(unit.data, sets.length);
      }
    } else if (unit.rasl && this.skipRasl) {
      return;
    } else if (!unit.key) {
      this.skipRasl = false;
    }
    this.unitBytes += unit.data.length;
    this.unitCount += 1;
    if (this.state.bytesTotal != null) {
      const frameCount = Math.round(this.state.bytesTotal / (this.unitBytes / this.unitCount));
      if (frameCount !== this.state.frameCount) this.update({ frameCount });
    }
    while (!signal.aborted && (this.frames.length >= MAX_BUFFERED_FRAMES || decoder.decodeQueueSize >= MAX_DECODE_QUEUE)) {
      await sleep(20);
    }
    if (signal.aborted) return;
    decoder.decode(new EncodedVideoChunk({ type: unit.key ? "key" : "delta", timestamp: unit.offset, data }));
  }

  private draw(frame: VideoFrame): void {
    if (this.canvas.width !== frame.displayWidth) this.canvas.width = frame.displayWidth;
    if (this.canvas.height !== frame.displayHeight) this.canvas.height = frame.displayHeight;
    this.canvas.getContext("2d")?.drawImage(frame, 0, 0);
  }

  private readonly render = (now: number): void => {
    if (this.closed) return;
    this.raf = requestAnimationFrame(this.render);
    const head = this.frames[0];
    if (this.state.status === "paused") {
      if (this.preview && head) {
        this.preview = false;
        this.draw(head.frame);
        this.update({ frame: head.index, position: head.frame.timestamp });
      }
      return;
    }
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
    this.preview = false;
    this.draw(shown.frame);
    this.update({ frame: shown.index, position: shown.frame.timestamp });
    shown.frame.close();
  };
}
