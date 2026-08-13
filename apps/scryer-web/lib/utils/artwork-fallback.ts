import type { CSSProperties } from "react";
import { blake3 } from "@noble/hashes/blake3.js";

export type ArtworkFallbackTone = "MOVIE" | "SERIES" | "ANIME" | "neutral";

const ARTWORK_FALLBACK_TONES: Record<
  ArtworkFallbackTone,
  { hue: number; spread: number; saturation: [number, number] }
> = {
  MOVIE: { hue: 30, spread: 56, saturation: [48, 68] },
  SERIES: { hue: 152, spread: 58, saturation: [44, 64] },
  ANIME: { hue: 278, spread: 62, saturation: [46, 68] },
  neutral: { hue: 214, spread: 64, saturation: [42, 62] },
};

const ARTWORK_FALLBACK_TEXT_ENCODER = new TextEncoder();

function fromByte(byte: number, min: number, max: number) {
  return min + (byte / 255) * (max - min);
}

function signedFromByte(byte: number, spread: number) {
  return fromByte(byte, -spread, spread);
}

function hsl(hue: number, saturation: number, lightness: number, alpha = 1) {
  const normalizedHue = ((Math.round(hue) % 360) + 360) % 360;
  return `hsl(${normalizedHue} ${Math.round(saturation)}% ${Math.round(lightness)}% / ${alpha.toFixed(2)})`;
}

export function artworkFallbackStyle(
  seed: string,
  tone: ArtworkFallbackTone,
): CSSProperties {
  const digest = blake3(
    ARTWORK_FALLBACK_TEXT_ENCODER.encode(seed.trim().toLocaleLowerCase()),
  );
  const toneConfig = ARTWORK_FALLBACK_TONES[tone];
  const hue = toneConfig.hue + signedFromByte(digest[0], toneConfig.spread);
  const accentHue = hue + signedFromByte(digest[1], 46);
  const shadowHue = hue + signedFromByte(digest[2], 24);
  const saturation = fromByte(
    digest[3],
    toneConfig.saturation[0],
    toneConfig.saturation[1],
  );
  const topLightness = fromByte(digest[4], 29, 44);
  const midLightness = fromByte(digest[5], 15, 27);
  const glowX = fromByte(digest[6], 28, 72);
  const glowAlpha = fromByte(digest[7], 0.3, 0.5);
  const secondaryHue = hue + signedFromByte(digest[8], 72);
  const secondaryAlpha = fromByte(digest[9], 0.1, 0.2);

  return {
    backgroundImage: [
      `radial-gradient(circle at ${glowX.toFixed(1)}% 17%, ${hsl(
        accentHue,
        saturation + 8,
        topLightness + 13,
        glowAlpha,
      )}, transparent 43%)`,
      `radial-gradient(circle at ${fromByte(digest[10], 18, 82).toFixed(1)}% ${fromByte(
        digest[11],
        58,
        86,
      ).toFixed(1)}%, ${hsl(
        secondaryHue,
        Math.max(38, saturation - 4),
        midLightness + 8,
        secondaryAlpha,
      )}, transparent 48%)`,
      `linear-gradient(180deg, ${hsl(hue, saturation, topLightness)} 0%, ${hsl(
        hue + signedFromByte(digest[12], 18),
        saturation - 1,
        midLightness,
      )} 58%, ${hsl(shadowHue, Math.max(32, saturation - 10), 8)} 100%)`,
    ].join(","),
  };
}
