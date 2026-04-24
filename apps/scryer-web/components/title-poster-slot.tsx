import * as React from "react";
import type { ComponentProps } from "react";
import { Loader2 } from "lucide-react";

import { TitlePoster } from "@/components/title-poster";

const HYDRATION_POSTER_GRACE_MS = 5 * 60 * 1000;

type TitlePosterSlotProps = Omit<ComponentProps<"img">, "src"> & {
  src?: string | null;
  sourceSrc?: string | null;
  metadataFetchedAt?: string | null;
  createdAt?: string | null;
  emptyLabel: string;
  placeholderClassName?: string;
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
  sourceSrc,
  metadataFetchedAt,
  createdAt,
  emptyLabel,
  className,
  placeholderClassName,
  alt,
  ...props
}: TitlePosterSlotProps) {
  const hasPoster = Boolean(src || sourceSrc);
  const showHydrationSpinner = useHydrationPosterGrace(
    hasPoster,
    metadataFetchedAt,
    createdAt,
  );

  if (hasPoster) {
    return (
      <TitlePoster
        src={src}
        sourceSrc={sourceSrc}
        alt={alt}
        className={className}
        {...props}
      />
    );
  }

  return (
    <div
      className={placeholderClassName ?? className}
      aria-label={alt}
      role="img"
    >
      {showHydrationSpinner ? (
        <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
      ) : (
        emptyLabel
      )}
    </div>
  );
}
