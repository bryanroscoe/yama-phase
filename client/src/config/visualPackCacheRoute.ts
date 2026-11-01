/** Workbox serializes this function into the worker: use no module closures. */
export function visualPackCacheRoute({ sameOrigin, url }: { sameOrigin: boolean; url: URL }): boolean {
  if (!sameOrigin) return false;
  const scope = (globalThis as unknown as { registration: { scope: string } }).registration.scope;
  const prefix = new URL("__visual-packs/sha256/", scope).pathname;
  return url.pathname.startsWith(prefix)
    && /^[0-9a-f]{64}\.(jpg|png|webp|svg)$/.test(url.pathname.slice(prefix.length));
}
