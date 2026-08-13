import * as React from "react";
import type { ComponentProps } from "react";

import { ArtworkFallback } from "@/components/artwork-fallback";
import { TitlePoster } from "@/components/title-poster";
import type { ArtworkFallbackTone } from "@/lib/utils/artwork-fallback";

const HYDRATION_POSTER_GRACE_MS = 5 * 60 * 1000;

type TitlePosterSlotProps = Omit<ComponentProps<"img">, "src"> & {
  src?: string | null;
  metadataFetchedAt?: string | null;
  createdAt?: string | null;
  emptyLabel: string;
  placeholderClassName?: string;
  fallbackTitle?: string | null;
  fallbackSubtitle?: string | number | null;
  fallbackTone?: ArtworkFallbackTone | null;
  fallbackShowText?: boolean;
};

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
    <ArtworkFallback
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
