import { describe, expect, it } from "vitest";
import { AccessUnitAssembler, AnnexBSplitter, hevcCodec, nalType, type NalUnit } from "./hevc";

// The first 64 bytes of a real HEVC Annex-B file: VPS, then the start of the SPS.
const HEAD = Uint8Array.from(
  "00000001 40010c01 ffff0160 00000300 90000003 00000300 5a970240 00000001 42010101 60000003 00900000 03000003 005aa006 c201e1c4 f965d290 84645fd0"
    .replace(/ /g, "")
    .match(/../g)!
    .map((h) => parseInt(h, 16)),
);

const types = (nals: NalUnit[]) => nals.map((n) => nalType(n.data));

describe("AnnexBSplitter", () => {
  it("splits NAL units across arbitrary chunk boundaries, keeping the last until flush", () => {
    const splitter = new AnnexBSplitter();
    const nals = [...splitter.push(HEAD.subarray(0, 30)), ...splitter.push(HEAD.subarray(30, 34))];
    expect(types(nals)).toEqual([32]);
    expect(nals[0].data).toHaveLength(24);
    expect(nals[0].offset).toBe(0);
    expect(splitter.push(HEAD.subarray(34))).toEqual([]);
    const rest = splitter.flush();
    expect(types(rest)).toEqual([33]);
    expect(rest[0].data[0]).toBe(0x42);
    expect(rest[0].offset).toBe(28);
  });

  it("starts mid-stream at the first start code, reporting stream offsets", () => {
    // Stream offset 1000 is HEAD[10], inside the VPS; the SPS start code is HEAD[28].
    const splitter = new AnnexBSplitter(1000);
    expect(splitter.push(HEAD.subarray(10))).toEqual([]);
    const rest = splitter.flush();
    expect(types(rest)).toEqual([33]);
    expect(rest[0].offset).toBe(1018);
  });
});

describe("hevcCodec", () => {
  it("builds the codec string from the SPS profile, tier, and level", () => {
    const splitter = new AnnexBSplitter();
    splitter.push(HEAD);
    expect(hevcCodec(splitter.flush()[0].data)).toBe("hev1.1.6.L90.90");
  });
});

describe("AccessUnitAssembler", () => {
  const at = (offset: number, ...bytes: number[]): NalUnit => ({ data: Uint8Array.from(bytes), offset });

  it("starts a picture at parameter sets or a first slice, flagging IRAP, RASL, and SPS", () => {
    const assembler = new AccessUnitAssembler();
    const sps = at(100, 33 << 1, 1, 0xaa);
    const cra = at(110, 21 << 1, 1, 0x80, 0x11);
    const craSecondSlice = at(120, 21 << 1, 1, 0x00, 0x22);
    const rasl = at(130, 8 << 1, 1, 0x80, 0x33);
    const units = [sps, cra, craSecondSlice, rasl].map((n) => assembler.push(n)).filter((u) => u != null);
    expect(units).toEqual([
      {
        key: true,
        rasl: false,
        hasSps: true,
        offset: 100,
        data: Uint8Array.from([0, 0, 0, 1, ...sps.data, 0, 0, 0, 1, ...cra.data, 0, 0, 0, 1, ...craSecondSlice.data]),
      },
    ]);
    expect(assembler.flush()).toEqual({
      key: false,
      rasl: true,
      hasSps: false,
      offset: 130,
      data: Uint8Array.from([0, 0, 0, 1, ...rasl.data]),
    });
    expect(assembler.flush()).toBeNull();
  });
});
