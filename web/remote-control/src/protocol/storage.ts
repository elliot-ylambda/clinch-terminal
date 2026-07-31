import type { Capability } from "../generated/types/Capability";
import type { DeviceId } from "../generated/types/DeviceId";
import type { PairingClaimReceipt } from "../generated/types/PairingClaimReceipt";

const DATABASE = "clinch-remote-control";
const STORE = "local-device";
const IDENTITY_KEY = "identity";
const PENDING_PAIRING_KEY = "pending-pairing";
const PREFERENCES_KEY = "preferences";

export interface DeviceIdentity {
  key: typeof IDENTITY_KEY;
  privateKey: CryptoKey;
  publicKeyP256Raw: string;
  deviceName: string;
  deviceId?: DeviceId;
  capabilities?: Capability[];
}

export interface MobilePreferences {
  key: typeof PREFERENCES_KEY;
  oneTapQuickInserts: boolean;
}

interface PendingPairingRecord {
  key: typeof PENDING_PAIRING_KEY;
  receipt: PairingClaimReceipt;
}

function database(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DATABASE, 1);
    request.onupgradeneeded = () => {
      if (!request.result.objectStoreNames.contains(STORE)) {
        request.result.createObjectStore(STORE, { keyPath: "key" });
      }
    };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error ?? new Error("Could not open local device store"));
  });
}

async function read<T>(key: string): Promise<T | undefined> {
  const db = await database();
  try {
    return await new Promise<T | undefined>((resolve, reject) => {
      const request = db.transaction(STORE, "readonly").objectStore(STORE).get(key);
      request.onsuccess = () => resolve(request.result as T | undefined);
      request.onerror = () => reject(request.error ?? new Error("Could not read local device"));
    });
  } finally {
    db.close();
  }
}

async function write<T>(value: T): Promise<void> {
  const db = await database();
  try {
    await new Promise<void>((resolve, reject) => {
      const transaction = db.transaction(STORE, "readwrite");
      transaction.objectStore(STORE).put(value);
      transaction.oncomplete = () => resolve();
      transaction.onerror = () => reject(transaction.error ?? new Error("Could not save local device"));
    });
  } finally {
    db.close();
  }
}

async function remove(key: string): Promise<void> {
  const db = await database();
  try {
    await new Promise<void>((resolve, reject) => {
      const transaction = db.transaction(STORE, "readwrite");
      transaction.objectStore(STORE).delete(key);
      transaction.oncomplete = () => resolve();
      transaction.onerror = () => reject(transaction.error ?? new Error("Could not update local device storage"));
    });
  } finally {
    db.close();
  }
}

export const loadIdentity = () => read<DeviceIdentity>(IDENTITY_KEY);
export const saveIdentity = (identity: DeviceIdentity) => write(identity);
export const clearIdentity = () => remove(IDENTITY_KEY);

export async function loadPendingPairing(): Promise<PairingClaimReceipt | undefined> {
  return (await read<PendingPairingRecord>(PENDING_PAIRING_KEY))?.receipt;
}

export const savePendingPairing = (receipt: PairingClaimReceipt) =>
  write<PendingPairingRecord>({ key: PENDING_PAIRING_KEY, receipt });

export const clearPendingPairing = () => remove(PENDING_PAIRING_KEY);

export async function loadPreferences(): Promise<MobilePreferences> {
  return (
    (await read<MobilePreferences>(PREFERENCES_KEY)) ?? {
      key: PREFERENCES_KEY,
      oneTapQuickInserts: false,
    }
  );
}

export const savePreferences = (preferences: MobilePreferences) => write(preferences);

export async function createIdentity(deviceName: string): Promise<DeviceIdentity> {
  const keys = (await crypto.subtle.generateKey(
    { name: "ECDSA", namedCurve: "P-256" },
    false,
    ["sign", "verify"],
  )) as CryptoKeyPair;
  if (keys.privateKey.extractable) {
    throw new Error("Browser created an exportable private device key");
  }
  const publicKey = new Uint8Array(await crypto.subtle.exportKey("raw", keys.publicKey));
  if (publicKey.length !== 65 || publicKey[0] !== 4) {
    throw new Error("Browser returned an unsupported P-256 public key");
  }
  const identity: DeviceIdentity = {
    key: IDENTITY_KEY,
    privateKey: keys.privateKey,
    publicKeyP256Raw: bytesToBase64(publicKey),
    deviceName,
  };
  await saveIdentity(identity);
  return identity;
}

export function bytesToBase64(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

export function base64ToBytes(value: string): Uint8Array {
  const binary = atob(value);
  return Uint8Array.from(binary, (character) => character.charCodeAt(0));
}
