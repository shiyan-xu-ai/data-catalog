import { describe, expect, it } from "vitest";
import { createLimiter } from "./limit";

describe("createLimiter", () => {
  it("never runs more than the limit at once and runs every task", async () => {
    const run = createLimiter(2);
    const gates = [1, 2, 3, 4].map(() => Promise.withResolvers<void>());
    const started: number[] = [];
    const results = gates.map((g, i) =>
      run(async () => {
        started.push(i);
        await g.promise;
        return i;
      }),
    );
    await Promise.resolve();
    expect(started).toEqual([0, 1]);
    gates[0].resolve();
    await results[0];
    await Promise.resolve();
    expect(started).toEqual([0, 1, 2]);
    gates.forEach((g) => g.resolve());
    expect(await Promise.all(results)).toEqual([0, 1, 2, 3]);
  });
  it("releases the slot when a task fails", async () => {
    const run = createLimiter(1);
    await expect(run(() => Promise.reject(new Error("x")))).rejects.toThrow("x");
    expect(await run(async () => 1)).toBe(1);
  });
});
