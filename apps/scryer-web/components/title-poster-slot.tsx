import * as React from "react";
import type { ComponentProps, CSSProperties } from "react";
import { blake3 } from "@noble/hashes/blake3.js";
import { Loader2 } from "lucide-react";

import { TitlePoster } from "@/components/title-poster";
import { cn } from "@/lib/utils";

const HYDRATION_POSTER_GRACE_MS = 5 * 60 * 1000;

type PosterFallbackTone = "MOVIE" | "SERIES" | "ANIME" | "neutral";

type TitlePosterSlotProps = Omit<ComponentProps<"img">, "src"> & {
  src?: string | null;
  metadataFetchedAt?: string | null;
  createdAt?: string | null;
  emptyLabel: string;
  placeholderClassName?: string;
  fallbackTitle?: string | null;
  fallbackSubtitle?: string | number | null;
  fallbackTone?: PosterFallbackTone | null;
  fallbackShowText?: boolean;
};

const POSTER_FALLBACK_TONES: Record<
  PosterFallbackTone,
  { hue: number; spread: number; saturation: [number, number] }
> = {
  MOVIE: { hue: 30, spread: 56, saturation: [48, 68] },
  SERIES: { hue: 152, spread: 58, saturation: [44, 64] },
  ANIME: { hue: 278, spread: 62, saturation: [46, 68] },
  neutral: { hue: 214, spread: 64, saturation: [42, 62] },
};

const POSTER_FALLBACK_TEXT_ENCODER = new TextEncoder();

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

function posterFallbackStyle(
  seed: string,
  tone: PosterFallbackTone,
): CSSProperties {
  const digest = blake3(
    POSTER_FALLBACK_TEXT_ENCODER.encode(seed.trim().toLocaleLowerCase()),
  );
  const toneConfig = POSTER_FALLBACK_TONES[tone];
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

function useHydrationPosterGrace(
  hasPoster: boolean,
  metadataFetchedAt: string | null | undefined,
  createdAt: string | null | undefined,
) {
  const createdAtMs = React.useMemo(() => {
    const parsed = createdAt ? Date.parse(createdAt) : Number.NaN;
    return Number.isFinite(parsed) ? parsed : null;
  }, [createdAt]);
  const [firstSeenUnhydratedMs, setFirstSeenUnhydratedMs] = React.useState<
    number | null
  >(null);

  React.useEffect(() => {
    if (hasPoster || metadataFetchedAt != null) {
      setFirstSeenUnhydratedMs(null);
      return;
    }
    setFirstSeenUnhydratedMs((current) => current ?? Date.now());
  }, [hasPoster, metadataFetchedAt]);
  const [nowMs, setNowMs] = React.useState(() => Date.now());

  const deadlineMs = React.useMemo(() => {
    if (hasPoster || metadataFetchedAt != null) {
      return null;
    }
    if (createdAtMs !== null) {
      const createdAtDeadlineMs = createdAtMs + HYDRATION_POSTER_GRACE_MS;
      if (createdAtDeadlineMs > nowMs) {
        return createdAtDeadlineMs;
      }
    }
    if (firstSeenUnhydratedMs === null) {
      return null;
    }
    return firstSeenUnhydratedMs + HYDRATION_POSTER_GRACE_MS;
  }, [createdAtMs, firstSeenUnhydratedMs, hasPoster, metadataFetchedAt, nowMs]);

  React.useEffect(() => {
    setNowMs(Date.now());
  }, [deadlineMs, hasPoster, metadataFetchedAt]);

  React.useEffect(() => {
    if (deadlineMs === null) {
      return;
    }
    const remainingMs = deadlineMs - Date.now();
    if (remainingMs <= 0) {
      return;
    }
    const timeoutId = window.setTimeout(() => {
      setNowMs(Date.now());
    }, remainingMs);
    return () => window.clearTimeout(timeoutId);
  }, [deadlineMs]);

  return deadlineMs !== null && nowMs < deadlineMs;
}

function TitlePosterFallback({
  className,
  ariaLabel,
  emptyLabel,
  showSpinner,
  title,
  subtitle,
  tone,
  showText,
}: {
  className?: string;
  ariaLabel?: string;
  emptyLabel: string;
  showSpinner: boolean;
  title?: string | null;
  subtitle?: string | number | null;
  tone?: PosterFallbackTone | null;
  showText: boolean;
}) {
  const displayTitle = title?.trim() || emptyLabel;
  const displaySubtitle =
    subtitle != null && `${subtitle}`.trim() !== "" ? `${subtitle}` : null;
  const resolvedTone = tone ?? "neutral";
  const gradientStyle = React.useMemo(
    () => posterFallbackStyle(displayTitle, resolvedTone),
    [displayTitle, resolvedTone],
  );

  return (
    <div
      className={cn(
        "relative isolate overflow-hidden",
        className,
      )}
      style={gradientStyle}
      aria-label={ariaLabel}
      role="img"
    >
      <div
        aria-hidden="true"
        className="absolute inset-0 bg-[radial-gradient(circle_at_50%_18%,rgba(255,255,255,0.16),transparent_36%),linear-gradient(180deg,rgba(255,255,255,0.04),rgba(0,0,0,0.36))]"
      />
      {showSpinner ? (
        <div className="relative z-10 flex h-full w-full items-center justify-center">
          <Loader2 className="h-5 w-5 animate-spin text-white/60" />
        </div>
      ) : showText ? (
        <div className="relative z-10 flex h-full w-full flex-col items-center justify-end px-3 pb-5 text-center">
          <p className="line-clamp-3 font-[var(--font-space-grotesk)] text-base font-bold leading-tight text-white drop-shadow-[0_1px_3px_rgba(0,0,0,0.55)]">
            {displayTitle}
          </p>
          {displaySubtitle ? (
            <p className="mt-1 text-sm font-medium text-white/72">
              {displaySubtitle}
            </p>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

export function TitlePosterSlot({
  src,
  metadataFetchedAt,
  createdAt,
  emptyLabel,
  fallbackTitle,
  fallbackSubtitle,
  fallbackTone,
  fallbackShowText = true,
  className,
  placeholderClassName,
  alt,
  onError,
  ...props
}: TitlePosterSlotProps) {
  const hasPoster = Boolean(src);
  const posterRenderKey = React.useMemo(
    () => [src ?? "", metadataFetchedAt ?? ""].join("|"),
    [metadataFetchedAt, src],
  );
  const [posterFailed, setPosterFailed] = React.useState(false);
  const showHydrationSpinner = useHydrationPosterGrace(
    hasPoster,
    metadataFetchedAt,
    createdAt,
  );

  React.useEffect(() => {
    setPosterFailed(false);
  }, [posterRenderKey]);

  if (hasPoster && !posterFailed) {
    return (
      <TitlePoster
        key={posterRenderKey}
        src={src}
        alt={alt}
        className={className}
        onError={(event) => {
          setPosterFailed(true);
          onError?.(event);
        }}
        {...props}
      />
    );
  }

  return (
    <TitlePosterFallback
      className={placeholderClassName ?? className}
      ariaLabel={alt}
      emptyLabel={emptyLabel}
      showSpinner={fallbackShowText && showHydrationSpinner}
      title={fallbackTitle}
      subtitle={fallbackSubtitle}
      tone={fallbackTone}
      showText={fallbackShowText}
    />
  );
}
