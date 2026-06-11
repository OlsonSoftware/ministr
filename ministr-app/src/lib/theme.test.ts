import { describe, expect, it } from "vitest";
import { resolveDark } from "./theme";

describe("theme resolution (System/Light/Dark triple)", () => {
  it("system follows the OS", () => {
    expect(resolveDark("system", true)).toBe(true);
    expect(resolveDark("system", false)).toBe(false);
  });

  it("explicit overrides win regardless of the OS", () => {
    expect(resolveDark("dark", false)).toBe(true);
    expect(resolveDark("light", true)).toBe(false);
  });
});
