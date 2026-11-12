import { describe, expect, it } from "vitest";
import { AccessUnitAssembler, AnnexBSplitter, hevcCodec, nalType } from "./hevc";

// The first 64 bytes of a real HEVC Annex-B file: VPS, then the start of the SPS.
const HEAD = Uint8Array.from(
  "00000001 40010c01 ffff0160 00000300 90000003 00000300 5a970240 00000001 42010101 60000003 00900000 03000003 005aa006 c201e1c4 f965d290 84645fd0"
    .replace(/ /g, "")
    .match(/../g)!
    .map((h) => parseInt(h, 16)),
);

const nal = (...bytes: number[]) => Uint8Array.from(bytes);

describe("AnnexBSplitter", () => {
  it("splits NAL units across arbitrary chunk boundaries, keeping the last until flush", () => {
    const splitter = new AnnexBSplitter();
    const nals = [...splitter.push(HEAD.subarray(0, 30)), ...splitter.push(HEAD.subarray(30, 34))];
    expect(nals.map(nalType)).toEqual([32]);
    expect(nals[0]).toHaveLength(24);
    expect(splitter.push(HEAD.subarray(34))).toEqual([]);
    const rest = splitter.flush();
    expect(rest.map(nalType)).toEqual([33]);
    expect(rest[0][0]).toBe(0x42);
  });
});

describe("hevcCodec", () => {
  it("builds the codec string from the SPS profile, tier, and level", () => {
    const splitter = new AnnexBSplitter();
    splitter.push(HEAD);
    expect(hevcCodec(splitter.flush()[0])).toBe("hev1.1.6.L90.90");
  });
});

describe("AccessUnitAssembler", () => {
  it("starts a picture at parameter sets or a first slice, and marks IRAP pictures as keys", () => {
    const assembler = new AccessUnitAssembler();
    const vps = nal(0x40, 1, 0xaa);
    const idr = nal(19 << 1, 1, 0x80, 0x11);
    const idrSecondSlice = nal(19 << 1, 1, 0x00, 0x22);
    const trail = nal(1 << 1, 1, 0x80, 0x33);
    const units = [vps, idr, idrSecondSlice, trail].map((n) => assembler.push(n)).filter((u) => u != null);
    expect(units).toHaveLength(1);
    expect(units[0]).toEqual({
      key: true,
      data: Uint8Array.from([0, 0, 0, 1, ...vps, 0, 0, 0, 1, ...idr, 0, 0, 0, 1, ...idrSecondSlice]),
    });
    expect(assembler.flush()).toEqual({ key: false, data: Uint8Array.from([0, 0, 0, 1, ...trail]) });
    expect(assembler.flush()).toBeNull();
  });
});
