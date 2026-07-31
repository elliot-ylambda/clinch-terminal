import { beforeEach, describe, expect, it } from "vitest";

import { apiUrl, clearPairingFragment, routeRoot, takePairingFragment, websocketUrl } from "./urls";

describe("private companion URLs", () => {
  beforeEach(() => {
    history.replaceState(null, "", "/clinch-remote-a1b2/pair");
  });

  it("keeps invitation material in the fragment parser", () => {
    expect(
      takePairingFragment({ pathname: "/pair", search: "", hash: "#invitation-id:one-time-secret" } as Location),
    ).toEqual({ invitationId: "invitation-id", secret: "one-time-secret" });
    expect(takePairingFragment({ pathname: "/pair", search: "", hash: "#invalid" } as Location)).toBeNull();
  });

  it("removes the secret without losing the private route", () => {
    history.replaceState(null, "", "/clinch-remote-a1b2/pair?source=qr#id:secret");
    clearPairingFragment();
    expect(location.pathname).toBe("/clinch-remote-a1b2/pair");
    expect(location.search).toBe("?source=qr");
    expect(location.hash).toBe("");
  });

  it("derives API and WebSocket paths under the random route", () => {
    expect(routeRoot()).toBe("/clinch-remote-a1b2");
    expect(apiUrl("auth/challenge")).toBe(`${location.origin}/clinch-remote-a1b2/api/v1/auth/challenge`);
    expect(websocketUrl()).toBe(`ws://${location.host}/clinch-remote-a1b2/ws`);
  });
});
