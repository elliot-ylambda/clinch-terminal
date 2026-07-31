import { describe, expect, it } from "vitest";

import { isPermanentAuthorizationError } from "./client";

describe("Remote Control reconnect authorization", () => {
  it("stops reconnecting for revoked or expired device authorization", () => {
    expect(isPermanentAuthorizationError("This phone's authorization was revoked.")).toBe(true);
    expect(isPermanentAuthorizationError("The authorization record expired.")).toBe(true);
  });

  it("keeps reconnecting for an offline Mac or a transient network failure", () => {
    expect(isPermanentAuthorizationError("Failed to fetch")).toBe(false);
    expect(isPermanentAuthorizationError("The Mac did not answer in time")).toBe(false);
  });
});
