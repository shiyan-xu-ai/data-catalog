import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

describe("release version", () => {
  it("matches between frontend/package.json and the Cargo workspace", () => {
    const pkg = JSON.parse(readFileSync(new URL("../../package.json", import.meta.url), "utf8")) as { version: string };
    const cargo = readFileSync(new URL("../../../Cargo.toml", import.meta.url), "utf8");
    const workspace = cargo.slice(cargo.indexOf("[workspace.package]"));
    const cargoVersion = /^version\s*=\s*"([^"]+)"/m.exec(workspace)?.[1];
    expect(cargoVersion).toBe(pkg.version);
  });
});
