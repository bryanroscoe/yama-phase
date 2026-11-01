import { useCallback, useEffect, useRef, useState, useSyncExternalStore, type ImgHTMLAttributes } from "react";
import { fetchImageArtist } from "../../services/scryfall.ts";
import { CardArtFallback } from "./CardArtFallback.tsx";
import { installedImageSource, subscribeInstalledImageSource } from "../../services/visualPacks/imageAttribution.ts";

export type ArtAttribution = "card" | "external";
interface Props extends ImgHTMLAttributes<HTMLImageElement> {
  attribution?: ArtAttribution;
  /** Visible fallback identity when the image itself is decorative. */
  fallbackName?: string;
  onAttributionUnavailable?(): void;
}

/** Hosted card crops display only with credit for the exact image being shown. */
export function CreditedArt({ src, alt = "", style, attribution = "card", fallbackName = alt, onAttributionUnavailable, ...props }: Props) {
  const hosted = import.meta.env.VITE_HOSTED_CARD_ART === "1" && attribution === "card";
  const installedSource = useSyncExternalStore(
    useCallback((listener) => hosted && src ? subscribeInstalledImageSource(src, listener) : () => {}, [hosted, src]),
    () => hosted && src ? installedImageSource(src) : null,
    () => null,
  );
  const attributionSrc = installedSource ?? src;
  const unavailable = useRef(onAttributionUnavailable);
  unavailable.current = onAttributionUnavailable;
  const [credit, setCredit] = useState<{ src: string; artist: string | null } | null>(null);
  useEffect(() => {
    if (!hosted || !attributionSrc) return;
    let current = true;
    void fetchImageArtist(attributionSrc).then(
      (artist) => { if (current) { setCredit({ src: attributionSrc, artist }); if (!artist) unavailable.current?.(); } },
      () => { if (current) { setCredit({ src: attributionSrc, artist: null }); unavailable.current?.(); } },
    );
    return () => { current = false; };
  }, [hosted, attributionSrc]);
  if (!hosted) return <img {...props} data-card-art src={src} alt={alt} style={style} />;
  const artist = credit && credit.src === attributionSrc ? credit.artist : null;
  if (!artist) return <CardArtFallback name={fallbackName} variant="artCrop" className={props.className ?? ""} style={style} />;
  return <>
    <img {...props} data-card-art src={src} alt={alt} style={{ ...style, filter: "none", opacity: 1 }} />
    <span data-art-credit className="pointer-events-none absolute inset-x-0 bottom-0 z-20 bg-black/85 px-1 text-center text-[9px] leading-tight text-white">{artist}</span>
  </>;
}
