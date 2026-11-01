// Browser content-addressed images retain their installed source, not a card-name guess.
// A shared byte URL with conflicting origins is ambiguous and must stay uncredited.
const installedSources = new Map<string, string | null>();
const listeners = new Map<string, Set<() => void>>();

export function rememberInstalledImageSource(src: string, sourceUrl: string | undefined): void {
  if (!sourceUrl) return;
  let source: string;
  try {
    const url = new URL(sourceUrl);
    url.search = "";
    url.hash = "";
    source = url.href;
  } catch { return; }
  const before = installedSources.get(src);
  if (!installedSources.has(src)) installedSources.set(src, source);
  else if (before !== source) installedSources.set(src, null);
  if (before !== installedSources.get(src)) listeners.get(src)?.forEach((listener) => listener());
}

export function installedImageSource(src: string): string | null {
  return installedSources.get(src) ?? null;
}

export function subscribeInstalledImageSource(src: string, listener: () => void): () => void {
  const subscribers = listeners.get(src) ?? new Set<() => void>();
  listeners.set(src, subscribers);
  subscribers.add(listener);
  return () => {
    subscribers.delete(listener);
    if (subscribers.size === 0) listeners.delete(src);
  };
}
