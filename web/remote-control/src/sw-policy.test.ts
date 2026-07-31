import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

describe("service worker privacy boundary", () => {
  it("precache-only navigation excludes API and WebSocket traffic", () => {
    const source = readFileSync(resolve(process.cwd(), "src/sw.ts"), "utf8");
    expect(source).toContain("precacheAndRoute(self.__WB_MANIFEST)");
    expect(source).toContain("/\\/api\\//");
    expect(source).toContain("/\\/ws$/");
    expect(source).not.toContain("NetworkFirst");
    expect(source).not.toContain("CacheFirst");
  });
});
