import type { Authenticate } from "../generated/types/Authenticate";
import type { AuthChallenge } from "../generated/types/AuthChallenge";
import type { Authenticated } from "../generated/types/Authenticated";
import type { PairingClaimReceipt } from "../generated/types/PairingClaimReceipt";
import type { PairingStatus } from "../generated/types/PairingStatus";
import { base64ToBytes, bytesToBase64, type DeviceIdentity, saveIdentity } from "./storage";
import { apiUrl, type PairingFragment } from "./urls";

async function post<T>(endpoint: string, body: unknown): Promise<T> {
  const response = await fetch(apiUrl(endpoint), {
    method: "POST",
    credentials: "same-origin",
    cache: "no-store",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  const payload = (await response.json().catch(() => null)) as T | { error?: { message?: string } } | null;
  if (!response.ok) {
    const message = payload && typeof payload === "object" && "error" in payload ? payload.error?.message : undefined;
    throw new Error(message ?? `Remote Control request failed (${response.status})`);
  }
  return payload as T;
}

export async function claimPhone(
  invitation: PairingFragment,
  identity: DeviceIdentity,
): Promise<PairingClaimReceipt> {
  return post<PairingClaimReceipt>("pair/claim", {
    invitation_id: invitation.invitationId,
    secret: invitation.secret,
    device_name: identity.deviceName,
    platform: /iPad/i.test(navigator.userAgent) ? "ipados" : /iPhone/i.test(navigator.userAgent) ? "ios" : "other",
    public_key_p256_raw: identity.publicKeyP256Raw,
  });
}

export async function waitForApproval(receipt: PairingClaimReceipt): Promise<PairingStatus> {
  const deadline = new Date(receipt.expires_at).getTime();
  while (Date.now() < deadline) {
    const status = await post<PairingStatus>("pair/status", {
      claim_id: receipt.claim_id,
      claim_secret: receipt.claim_secret,
    });
    if (status.status !== "pending") return status;
    await new Promise((resolve) => setTimeout(resolve, 3_000));
  }
  return { status: "expired" };
}

export async function authenticate(identity: DeviceIdentity, lastSeenSequence: number): Promise<Authenticated> {
  if (!identity.deviceId) throw new Error("This phone has not been approved yet");
  const challenge = await post<AuthChallenge>("auth/challenge", { device_id: identity.deviceId });
  const challengeBytes = Uint8Array.from(base64ToBytes(challenge.challenge));
  const signature = new Uint8Array(
    await crypto.subtle.sign(
      { name: "ECDSA", hash: "SHA-256" },
      identity.privateKey,
      challengeBytes,
    ),
  );
  if (signature.length !== 64) throw new Error("Browser returned an unsupported ECDSA signature");
  const request: Authenticate = {
    device_id: identity.deviceId,
    challenge_id: challenge.id,
    signature: bytesToBase64(signature),
    last_seen_sequence: lastSeenSequence,
  };
  return post<Authenticated>("auth/authenticate", request);
}

export async function finishPairing(
  identity: DeviceIdentity,
  status: Extract<PairingStatus, { status: "approved" }>,
): Promise<DeviceIdentity> {
  const approved: DeviceIdentity = {
    ...identity,
    deviceId: status.data.device_id,
    capabilities: status.data.capabilities,
  };
  await saveIdentity(approved);
  return approved;
}
