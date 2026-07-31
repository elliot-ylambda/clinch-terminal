export interface PairingFragment {
  invitationId: string;
  secret: string;
}

export function takePairingFragment(locationLike: Pick<Location, "hash" | "pathname" | "search">): PairingFragment | null {
  const value = locationLike.hash.startsWith("#") ? locationLike.hash.slice(1) : locationLike.hash;
  if (!value) return null;
  const separator = value.indexOf(":");
  if (separator <= 0 || separator === value.length - 1) return null;
  return { invitationId: value.slice(0, separator), secret: value.slice(separator + 1) };
}

export function clearPairingFragment(): void {
  if (location.hash) history.replaceState(null, "", `${location.pathname}${location.search}`);
}

export function routeRoot(pathname = location.pathname): string {
  const withoutPair = pathname.replace(/\/pair\/?$/, "");
  return withoutPair.endsWith("/") ? withoutPair.slice(0, -1) : withoutPair;
}

export function apiUrl(endpoint: string): string {
  return `${location.origin}${routeRoot()}/api/v1/${endpoint.replace(/^\//, "")}`;
}

export function websocketUrl(): string {
  const protocol = location.protocol === "https:" ? "wss:" : "ws:";
  return `${protocol}//${location.host}${routeRoot()}/ws`;
}
