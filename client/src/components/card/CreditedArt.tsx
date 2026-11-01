import { useEffect, useRef, useState, type ImgHTMLAttributes } from "react";
import { fetchImageArtist } from "../../services/scryfall.ts";
import { CardArtFallback } from "./CardArtFallback.tsx";

export type ArtAttribution = "card" | "external";
interface Props extends ImgHTMLAttributes<HTMLImageElement> {
  attribution?: ArtAttribution;
  onAttributionUnavailable?(): void;
}

/** Hosted card crops display only with credit for the exact image being shown. */
export function CreditedArt({ src, alt = "", style, attribution = "card", onAttributionUnavailable, ...props }: Props) {
  const hosted = import.meta.env.VITE_HOSTED_CARD_ART === "1" && attribution === "card";
  const unavailable = useRef(onAttributionUnavailable);
  unavailable.current = onAttributionUnavailable;
  const [credit, setCredit] = useState<{ src: string; artist: string | null } | null>(null);
  useEffect(() => {
    if (!hosted || !src) return;
    let current = true;
    void fetchImageArtist(src).then(
      (artist) => { if (current) { setCredit({ src, artist }); if (!artist) unavailable.current?.(); } },
      () => { if (current) { setCredit({ src, artist: null }); unavailable.current?.(); } },
    );
    return () => { current = false; };
  }, [hosted, src]);
  if (!hosted) return <img {...props} data-card-art src={src} alt={alt} style={style} />;
  const artist = credit && credit.src === src ? credit.artist : null;
  if (!artist) return <CardArtFallback name={alt} variant="artCrop" className={props.className ?? ""} style={style} />;
  return <>
    <img {...props} data-card-art src={src} alt={alt} style={{ ...style, filter: "none", opacity: 1 }} />
    <span data-art-credit className="pointer-events-none absolute inset-x-0 bottom-0 z-20 bg-black/85 px-1 text-center text-[9px] leading-tight text-white">{artist}</span>
  </>;
}
