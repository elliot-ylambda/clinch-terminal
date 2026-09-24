import { describe, expect, it } from "vitest";

import { defaultDeviceName, pairingCode } from "./pairing";

describe("pairing matching code", () => {
  it("shows the head of the key fingerprint as XXXX-XXXX", () => {
    expect(pairingCode("0123456789abcdef".repeat(4))).toBe("0123-4567");
    expect(pairingCode("deadbeef00")).toBe("DEAD-BEEF");
  });
});

describe("default device name", () => {
  const ua = {
    iphoneSafari: "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1",
    iphoneChrome: "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) CriOS/130.0.0.0 Mobile/15E148 Safari/604.1",
    iphoneFirefox: "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) FxiOS/132.0 Mobile/15E148 Safari/605.1.15",
    iphoneEdge: "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 EdgiOS/130.0.0.0 Mobile/15E148 Safari/605.1.15",
    ipadDesktop: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15",
    androidChrome: "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Mobile Safari/537.36",
    androidEdge: "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Mobile Safari/537.36 EdgA/130.0.0.0",
  };

  it("names the device and browser", () => {
    expect(defaultDeviceName(ua.iphoneSafari, 5)).toBe("iPhone · Safari");
    expect(defaultDeviceName(ua.iphoneChrome, 5)).toBe("iPhone · Chrome");
    expect(defaultDeviceName(ua.iphoneFirefox, 5)).toBe("iPhone · Firefox");
    expect(defaultDeviceName(ua.iphoneEdge, 5)).toBe("iPhone · Edge");
    expect(defaultDeviceName(ua.androidChrome, 5)).toBe("Android · Chrome");
    expect(defaultDeviceName(ua.androidEdge, 5)).toBe("Android · Edge");
  });

  it("recognizes iPadOS in desktop mode by its touchscreen", () => {
    expect(defaultDeviceName(ua.ipadDesktop, 5)).toBe("iPad · Safari");
    expect(defaultDeviceName(ua.ipadDesktop, 0)).toBe("Mobile browser");
  });
});
