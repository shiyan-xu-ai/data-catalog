/** Parsing of raw H.265 (HEVC) Annex-B byte streams: start-code delimited NAL units, grouped into
 *  access units (one picture each) for WebCodecs, plus the codec string from the SPS. */

/** One picture's NAL units in Annex-B form; `key` when it holds an IRAP (random access) slice. */
export interface AccessUnit {
  data: Uint8Array;
  key: boolean;
}

export const NAL_SPS = 33;

export function nalType(nal: Uint8Array): number {
  return (nal[0] >> 1) & 0x3f;
}

function concat(a: Uint8Array, b: Uint8Array): Uint8Array {
  if (a.length === 0) return b;
  const out = new Uint8Array(a.length + b.length);
  out.set(a);
  out.set(b, a.length);
  return out;
}

/** Start codes in `data` as `[code, payload]` offsets; a 4-byte code starts at its leading zero. */
function startCodes(data: Uint8Array): [number, number][] {
  const found: [number, number][] = [];
  for (let i = 0; i + 2 < data.length; i++) {
    if (data[i] === 0 && data[i + 1] === 0 && data[i + 2] === 1) {
      found.push([i > 0 && data[i - 1] === 0 ? i - 1 : i, i + 3]);
      i += 2;
    }
  }
  return found;
}

/** Splits a byte stream arriving in arbitrary chunks into NAL units (without start codes). The
 *  last unit stays buffered until the next start code or `flush`. */
export class AnnexBSplitter {
  private pending: Uint8Array = new Uint8Array(0);

  push(chunk: Uint8Array): Uint8Array[] {
    const data = concat(this.pending, chunk);
    const codes = startCodes(data);
    const nals: Uint8Array[] = [];
    for (let i = 0; i + 1 < codes.length; i++) nals.push(data.subarray(codes[i][1], codes[i + 1][0]));
    this.pending = codes.length > 0 ? data.subarray(codes[codes.length - 1][0]) : data;
    return nals;
  }

  flush(): Uint8Array[] {
    const codes = startCodes(this.pending);
    const last = codes.length > 0 ? this.pending.subarray(codes[codes.length - 1][1]) : null;
    this.pending = new Uint8Array(0);
    return last && last.length > 0 ? [last] : [];
  }
}

/** Non-VCL NAL types that open a new access unit when they follow a picture's slices: VPS, SPS,
 *  PPS, access unit delimiter, prefix SEI, and reserved/unspecified prefix types (H.265 7.4.2.4.4). */
function startsAccessUnit(type: number): boolean {
  return (type >= 32 && type <= 35) || type === 39 || (type >= 41 && type <= 44) || (type >= 48 && type <= 55);
}

/** Groups NAL units in decoding order into access units. */
export class AccessUnitAssembler {
  private nals: Uint8Array[] = [];
  private hasSlice = false;
  private key = false;

  /** Adds `nal`; returns the access unit it completes, if any. */
  push(nal: Uint8Array): AccessUnit | null {
    const type = nalType(nal);
    const slice = type < 32;
    // A slice with first_slice_segment_in_pic_flag set begins a new picture.
    const opens = slice ? (nal[2] & 0x80) !== 0 : startsAccessUnit(type);
    const done = this.hasSlice && opens ? this.take() : null;
    this.nals.push(nal);
    if (slice) {
      this.hasSlice = true;
      if (type >= 16 && type <= 21) this.key = true;
    }
    return done;
  }

  /** The buffered access unit at the end of the stream, if it holds a picture. */
  flush(): AccessUnit | null {
    return this.hasSlice ? this.take() : null;
  }

  private take(): AccessUnit {
    const size = this.nals.reduce((sum, nal) => sum + 4 + nal.length, 0);
    const data = new Uint8Array(size);
    let at = 0;
    for (const nal of this.nals) {
      data.set([0, 0, 0, 1], at);
      data.set(nal, at + 4);
      at += 4 + nal.length;
    }
    const unit = { data, key: this.key };
    this.nals = [];
    this.hasSlice = false;
    this.key = false;
    return unit;
  }
}

/** Removes emulation-prevention bytes (`00 00 03` -> `00 00`). */
function rbsp(bytes: Uint8Array): Uint8Array {
  const out: number[] = [];
  let zeros = 0;
  for (const b of bytes) {
    if (zeros >= 2 && b === 3) {
      zeros = 0;
      continue;
    }
    out.push(b);
    zeros = b === 0 ? zeros + 1 : 0;
  }
  return Uint8Array.from(out);
}

function reverseBits32(n: number): number {
  let out = 0;
  for (let i = 0; i < 32; i++) out = (out << 1) | ((n >>> i) & 1);
  return out >>> 0;
}

/** The WebCodecs codec string (ISO/IEC 14496-15 annex E, e.g. `hev1.1.6.L90.90`) from an SPS
 *  NAL unit's general profile_tier_level. */
export function hevcCodec(sps: Uint8Array): string {
  // After the 2-byte NAL header: one byte of VPS id, sub-layer count, and nesting flag, then
  // profile space (2 bits), tier (1), profile (5), 32 compatibility flags, 48 constraint bits, level.
  const ptl = rbsp(sps.subarray(2, 40)).subarray(1);
  const space = ptl[0] >> 6;
  const tier = (ptl[0] >> 5) & 1;
  const profile = ptl[0] & 0x1f;
  const compat = ((ptl[1] << 24) | (ptl[2] << 16) | (ptl[3] << 8) | ptl[4]) >>> 0;
  const constraints = Array.from(ptl.subarray(5, 11));
  while (constraints.length > 0 && constraints[constraints.length - 1] === 0) constraints.pop();
  const hex = (n: number) => n.toString(16).toUpperCase();
  return [
    "hev1",
    `${["", "A", "B", "C"][space]}${profile}`,
    hex(reverseBits32(compat)),
    `${tier ? "H" : "L"}${ptl[11]}`,
    ...constraints.map(hex),
  ].join(".");
}
