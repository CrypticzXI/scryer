import * as React from "react";
import { Button } from "@/components/ui/button";
import type { ViewId, Translate } from "@/components/root/types";
import type { TitleRecord } from "@/lib/types";
import type { ParsedQualityProfile } from "@/lib/types/quality-profiles";
import { cn } from "@/lib/utils";
import {
  boxedActionButtonBaseClass,
  boxedActionButtonToneClass,
  type BoxedActionButtonTone,
} from "@/lib/utils/action-button-styles";

const QP_TAG_PREFIX = "scryer:quality-profile:";

export type TitleTableSortKey =
  | "name"
  | "monitored"
  | "quality"
  | "episodes"
  | "status"
  | "size";
export type TitleTableSortDirection = "asc" | "desc";

export function resolveOverviewTargetView(view: string): ViewId {
  if (view === "movies") {
    return "movies";
  }
  if (view === "anime") {
    return "anime";
  }
  return "series";
}

export function formatProfileLabel(
  value: string | null | undefined,
): string | null {
  const trimmed = value?.trim();
  if (!trimmed) {
    return null;
  }
  if (trimmed.toLowerCase() === "4k") {
    return "4K";
  }
  if (/^\d{3,4}p$/i.test(trimmed)) {
    return trimmed.toUpperCase();
  }
  return trimmed;
}

export function bytesToReadable(raw: number | null | undefined) {
  if (!raw || raw <= 0) {
    return "—";
  }
  if (raw > 1024 * 1024 * 1024) {
    return `${(raw / (1024 * 1024 * 1024)).toFixed(2)} GB`;
  }
  if (raw > 1024 * 1024) {
    return `${(raw / (1024 * 1024)).toFixed(2)} MB`;
  }
  if (raw > 1024) {
    return `${(raw / 1024).toFixed(2)} KB`;
  }
  return `${raw} B`;
}

export function formatEpisodeProgress(
  ownedEpisodes: number | null | undefined,
  monitoredEpisodes: number | null | undefined,
) {
  if (typeof monitoredEpisodes !== "number") {
    return "—";
  }

  if (monitoredEpisodes <= 0) {
    return "0 / 0";
  }

  const owned =
    typeof ownedEpisodes === "number" && ownedEpisodes >= 0 ? ownedEpisodes : 0;
  return `${owned} / ${monitoredEpisodes}`;
}

function normalizeTitleForUiSort(value: string) {
  const trimmed = value.trim();
  if (!trimmed) {
    return trimmed;
  }
  const withoutArticle = trimmed.replace(/^(a|an|the)\s+/i, "");
  return withoutArticle.trim() || trimmed;
}

function compareText(left: string, right: string) {
  return left.localeCompare(right, undefined, {
    sensitivity: "base",
    numeric: true,
  });
}

function compareTitleText(left: string, right: string) {
  const normalizedLeft = normalizeTitleForUiSort(left);
  const normalizedRight = normalizeTitleForUiSort(right);
  const normalizedDelta = compareText(normalizedLeft, normalizedRight);
  if (normalizedDelta !== 0) {
    return normalizedDelta;
  }
  return compareText(left, right);
}

function compareMaybeText(
  left: string | null | undefined,
  right: string | null | undefined,
) {
  const normalizedLeft = left?.trim() ?? "";
  const normalizedRight = right?.trim() ?? "";
  if (!normalizedLeft && !normalizedRight) {
    return 0;
  }
  if (!normalizedLeft) {
    return 1;
  }
  if (!normalizedRight) {
    return -1;
  }
  return compareText(normalizedLeft, normalizedRight);
}

function compareBooleans(left: boolean, right: boolean) {
  return Number(left) - Number(right);
}

function compareNumbers(
  left: number | null | undefined,
  right: number | null | undefined,
) {
  const normalizedLeft = left ?? Number.NEGATIVE_INFINITY;
  const normalizedRight = right ?? Number.NEGATIVE_INFINITY;
  return normalizedLeft - normalizedRight;
}

function compareEpisodeProgressValues(left: TitleRecord, right: TitleRecord) {
  const leftOwned = left.episodesOwned ?? 0;
  const rightOwned = right.episodesOwned ?? 0;
  const leftTarget = left.episodesMonitored ?? left.episodesTotal ?? 0;
  const rightTarget = right.episodesMonitored ?? right.episodesTotal ?? 0;
  const leftRatio =
    leftTarget > 0 ? leftOwned / leftTarget : Number.NEGATIVE_INFINITY;
  const rightRatio =
    rightTarget > 0 ? rightOwned / rightTarget : Number.NEGATIVE_INFINITY;

  const ratioDelta = leftRatio - rightRatio;
  if (ratioDelta !== 0) {
    return ratioDelta;
  }

  const ownedDelta = leftOwned - rightOwned;
  if (ownedDelta !== 0) {
    return ownedDelta;
  }

  return leftTarget - rightTarget;
}

function resolveTitleProfileName(
  item: TitleRecord,
  profiles: ParsedQualityProfile[],
  fallback: string | null,
): string | null {
  const tag = item.tags?.find((tagValue) => tagValue.startsWith(QP_TAG_PREFIX));
  if (tag) {
    const id = tag.slice(QP_TAG_PREFIX.length);
    const match = profiles.find((profile) => profile.id === id);
    if (match) {
      return match.name;
    }
    return formatProfileLabel(id);
  }

  return formatProfileLabel(fallback) ?? fallback;
}

export function resolveDisplayedQualityLabel(
  item: TitleRecord,
  profiles: ParsedQualityProfile[],
  fallback: string | null,
  unknownLabel: string,
) {
  return (
    resolveTitleProfileName(item, profiles, fallback) || unknownLabel
  );
}

export function sortTitlesForTable({
  titles,
  sortKey,
  sortDirection,
  qualityProfiles,
  resolvedProfileName,
  qualityProfilesLoading,
  t,
}: {
  titles: TitleRecord[];
  sortKey: TitleTableSortKey;
  sortDirection: TitleTableSortDirection;
  qualityProfiles: ParsedQualityProfile[];
  resolvedProfileName: string | null;
  qualityProfilesLoading: boolean;
  t: Translate;
}): TitleRecord[] {
  const factor = sortDirection === "asc" ? 1 : -1;
  const getStatusSortLabel = (item: TitleRecord) => {
    const normalized = item.contentStatus?.toLowerCase() ?? "";
    switch (normalized) {
      case "ended":
        return t("title.ended");
      case "upcoming":
        return t("title.upcoming");
      case "continuing":
        return t("title.continuing");
      default:
        return "";
    }
  };

  return [...titles].sort((left, right) => {
    const delta = (() => {
      switch (sortKey) {
        case "name":
          return compareTitleText(left.name, right.name);
        case "monitored":
          return compareBooleans(left.monitored, right.monitored);
        case "quality":
          if (qualityProfilesLoading) {
            return 0;
          }
          return compareMaybeText(
            resolveDisplayedQualityLabel(
              left,
              qualityProfiles,
              resolvedProfileName,
              t("label.unknown"),
            ),
            resolveDisplayedQualityLabel(
              right,
              qualityProfiles,
              resolvedProfileName,
              t("label.unknown"),
            ),
          );
        case "episodes":
          return compareEpisodeProgressValues(left, right);
        case "status":
          return compareMaybeText(
            getStatusSortLabel(left),
            getStatusSortLabel(right),
          );
        case "size":
          return compareNumbers(left.sizeBytes, right.sizeBytes);
        default:
          return 0;
      }
    })();

    if (delta !== 0) {
      return delta * factor;
    }

    return compareTitleText(left.name, right.name);
  });
}

export function defaultSortDirectionForTitleKey(
  key: TitleTableSortKey,
): TitleTableSortDirection {
  switch (key) {
    case "monitored":
    case "episodes":
    case "size":
      return "desc";
    default:
      return "asc";
  }
}

export function StatusBadge({
  status,
  t,
}: {
  status?: string | null;
  t: Translate;
}) {
  const normalized = status?.toLowerCase() ?? "";
  if (normalized === "ended") {
    return (
      <span className="rounded bg-zinc-700/60 px-2 py-0.5 text-xs text-zinc-300">
        {t("title.ended")}
      </span>
    );
  }
  if (normalized === "upcoming") {
    return (
      <span className="rounded bg-blue-900/50 px-2 py-0.5 text-xs text-blue-300">
        {t("title.upcoming")}
      </span>
    );
  }
  if (normalized === "continuing") {
    return (
      <span className="rounded bg-emerald-900/50 px-2 py-0.5 text-xs text-emerald-300">
        {t("title.continuing")}
      </span>
    );
  }
  return null;
}

export function TitleTableActionButton({
  label,
  tone,
  className,
  children,
  ...props
}: React.ComponentProps<typeof Button> & {
  label: string;
  tone: BoxedActionButtonTone;
}) {
  return (
    <Button
      type="button"
      size="icon-sm"
      variant="secondary"
      title={label}
      aria-label={label}
      className={cn(
        boxedActionButtonBaseClass,
        boxedActionButtonToneClass[tone],
        className,
      )}
      {...props}
    >
      {children}
    </Button>
  );
}
