import * as React from "react";
import { useLocation } from "react-router-dom";
import {
  ArrowDown,
  ArrowUp,
  Check,
  ChevronDown,
  ChevronRight,
  ClipboardList,
  Columns3,
  Edit,
  Eye,
  EyeOff,
  FolderOpen,
  LayoutGrid,
  LayoutList,
  Loader2,
  PanelLeftOpen,
  Pencil,
  Plus,
  RefreshCw,
  Search,
  Send,
  Sparkles,
  Table as TableIcon,
  Trash2,
  X,
  Zap,
} from "lucide-react";
import {
  AnidbExternalLink,
  AnilistExternalLink,
  ImdbExternalLink,
  MalExternalLink,
  TmdbExternalLink,
  TvdbMovieExternalLink,
  TvdbSeriesExternalLink,
} from "@/components/common/external-media-links";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { Input } from "@/components/ui/input";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { LibraryMultiSelect } from "@/components/common/library-multi-select";
import {
  MediaFilesOnDiskPanel,
  type MediaFileOnDisk,
} from "@/components/common/media-files-on-disk-panel";
import {
  MediaRenamePlanPanel,
  type MediaRenamePlan,
} from "@/components/common/media-rename-plan-panel";
import { TitleHistoryModal } from "@/components/common/title-history-modal";
import {
  SearchResultBuckets,
  type ReleaseSearchSortDirection,
  type ReleaseSearchSortKey,
} from "@/components/common/release-search-results";
import { TitlePosterSlot } from "@/components/title-poster-slot";
import type {
  ContentSettingsSection,
  OverviewTitleTarget,
  Translate,
  ViewId,
} from "@/components/root/types";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import type {
  DownloadClientRecord,
  DownloadClientRoutingEntry,
  IndexerCategoryRoutingSettings,
  IndexerRecord,
  LibraryScanSummary,
  LibraryRecord,
  LibrarySettingsDraft,
  LibrarySettingsRecord,
  NzbgetCategoryRoutingSettings,
  Release,
  TitleReleaseBlocklistEntry,
  TitleRecord,
  DiscoveryItem,
} from "@/lib/types";
import type { ImportMode } from "@/lib/types/settings";
import type { ExternalSubtitleRecord } from "@/lib/types/subtitles";
import type { ViewCategoryId } from "./media-content/indexer-category-picker";
import { MediaLibrarySettingsPanel } from "./media-content/media-library-settings-panel";
import { IndexerRoutingPanel } from "./media-content/indexer-routing-panel";
import { DownloadClientRoutingPanel } from "./media-content/download-client-routing-panel";
import { GeneralSettingsPanel } from "./media-content/general-settings-panel";
import { QualitySettingsPanel } from "./media-content/quality-settings-panel";
import { RenameSettingsPanel } from "./media-content/rename-settings-panel";
import { AddTitleForm } from "./media-content/add-title-form";
import { PosterGrid } from "./media-content/poster-grid";
import { TitleTable } from "./media-content/title-table";
import { CompactTitleTable } from "./media-content/compact-title-table";
import {
  TitleTableActionButton,
  DEFAULT_TITLE_TABLE_VISIBLE_COLUMNS,
  TITLE_TABLE_COLUMN_KEYS,
  bytesToReadable,
  formatTitleDate,
  resolveDisplayedQualityLabel,
  type TitleTableColumnKey,
  type TitleTableSortDirection,
  type TitleTableSortKey,
  type TitleTableVisibleColumns,
} from "./media-content/title-table-shared";
import { titleOverviewViewModeId } from "@/lib/utils/dom-ids";
import {
  hasActiveTitleQuickFilters,
  TitleQuickFilterBar,
  type TitleQuickFilterCounts,
  type TitleQuickFilters,
} from "./media-content/title-quick-filters";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import type { RuleSetRecord } from "@/lib/types/rule-sets";
import type {
  FacetScoringPersonaSelectionRecord,
  ParsedQualityProfileEntry,
  ScoringPersonaId,
} from "@/lib/types/quality-profiles";
import { buildViewPath } from "@/lib/utils/routing";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";
import { cn } from "@/lib/utils";
import { persistOverviewWindowScroll } from "@/lib/hooks/use-overview-window-scroll-restoration";
import { releaseSupportsAdditionalFileQueue } from "@/lib/utils/release-queue-scope";
import type { LocalPathStyle } from "@/lib/utils/local-path-style";
import type { ContentViewMode } from "./media-content/content-view-mode";
import { localizedTitleStatus } from "./overview-localization";
import { SeriesOverviewContainer } from "@/components/containers/series-overview-container";

type Facet = "movie" | "series" | "anime";

function titleTableColumnLabel(
  key: TitleTableColumnKey,
  t: Translate,
): string {
  switch (key) {
    case "library":
      return t("title.table.library");
    case "monitored":
      return t("title.table.monitored");
    case "quality":
      return t("title.table.qualityTier");
    case "episodes":
      return t("title.table.episodes");
    case "size":
      return t("title.table.size");
    case "added":
      return t("title.contextAdded");
    case "actions":
      return t("label.actions");
  }
}

type ParsedQualityProfile = {
  id: string;
  name: string;
};

const TITLE_OVERVIEW_PANE_MIN_WIDTH = 700;
const TITLE_OVERVIEW_PANE_MAX_WIDTH = 1030;
const TITLE_WORKSPACE_PANE_GAP = 16;
const TITLE_POSTER_GRID_MIN_COLUMN_WIDTH = 150;
const SELECTED_POSTER_INLINE_MIN_WIDTH =
  TITLE_OVERVIEW_PANE_MIN_WIDTH +
  TITLE_WORKSPACE_PANE_GAP +
  TITLE_POSTER_GRID_MIN_COLUMN_WIDTH;

const TITLE_TABLE_COLUMN_SHEDDING_TIERS: readonly {
  maxWidth: number;
  columns: readonly TitleTableColumnKey[];
}[] = [
  { maxWidth: 1280, columns: ["added"] },
  { maxWidth: 1180, columns: ["episodes"] },
  { maxWidth: 1040, columns: ["library"] },
  { maxWidth: 920, columns: ["quality"] },
  { maxWidth: 780, columns: ["actions"] },
  { maxWidth: 640, columns: ["size"] },
  { maxWidth: 520, columns: ["monitored"] },
];

function clampPaneWidth(width: number, minWidth: number, maxWidth: number) {
  return Math.min(Math.max(width, minWidth), maxWidth);
}

function resolveTitleTablePaneWidth({
  collectionViewMode,
  contextPanelAvailable,
  layoutWidth,
  selectedTitleLayoutActive,
  selectedTitleListInlineActive,
  selectedTitlePosterLayoutActive,
}: {
  collectionViewMode: ContentViewMode;
  contextPanelAvailable: boolean;
  layoutWidth: number | null;
  selectedTitleLayoutActive: boolean;
  selectedTitleListInlineActive: boolean;
  selectedTitlePosterLayoutActive: boolean;
}) {
  if (layoutWidth == null || collectionViewMode === "poster") {
    return layoutWidth;
  }

  const panelInline = selectedTitleLayoutActive
    ? selectedTitleListInlineActive || selectedTitlePosterLayoutActive
    : contextPanelAvailable;

  if (!panelInline) {
    return layoutWidth;
  }

  const panelWidth = clampPaneWidth(
    layoutWidth * 0.5,
    TITLE_OVERVIEW_PANE_MIN_WIDTH,
    TITLE_OVERVIEW_PANE_MAX_WIDTH,
  );

  return Math.max(layoutWidth - panelWidth - TITLE_WORKSPACE_PANE_GAP, 0);
}

function resolveEffectiveTitleTableColumns(
  visibleColumns: TitleTableVisibleColumns,
  tablePaneWidth: number | null,
  selectedTitleInlineActive: boolean,
): TitleTableVisibleColumns {
  const nextColumns = { ...visibleColumns };
  const hiddenColumns = new Set<TitleTableColumnKey>();

  if (selectedTitleInlineActive) {
    hiddenColumns.add("added");
    hiddenColumns.add("actions");
    hiddenColumns.add("library");
    hiddenColumns.add("quality");
    hiddenColumns.add("episodes");
  }

  if (tablePaneWidth != null) {
    for (const tier of TITLE_TABLE_COLUMN_SHEDDING_TIERS) {
      if (tablePaneWidth < tier.maxWidth) {
        for (const column of tier.columns) {
          hiddenColumns.add(column);
        }
      }
    }
  }

  for (const column of hiddenColumns) {
    nextColumns[column] = false;
  }

  return nextColumns;
}

type QualityProfileOption = {
  value: string;
  label: string;
};

type TvdbSearchItem = MetadataTvdbSearchItem;

type ScopeRoutingRecord = Record<string, NzbgetCategoryRoutingSettings>;
type IndexerRoutingRecord = Record<string, IndexerCategoryRoutingSettings>;

function formatQualityProfileFallback(
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

function mediaTitleLabel(view: ViewId, t: Translate): string {
  if (view === "movies") {
    return t("title.manageMovies");
  }
  if (view === "anime") {
    return t("nav.anime");
  }
  return t("nav.series");
}

function useMinViewportWidth(query: string) {
  const [matches, setMatches] = React.useState(() =>
    typeof window === "undefined" ? false : window.matchMedia(query).matches,
  );

  React.useEffect(() => {
    if (typeof window === "undefined") {
      return;
    }

    const mediaQuery = window.matchMedia(query);
    const handleChange = () => setMatches(mediaQuery.matches);
    handleChange();
    mediaQuery.addEventListener("change", handleChange);
    return () => {
      mediaQuery.removeEventListener("change", handleChange);
    };
  }, [query]);

  return matches;
}

function useMeasuredElementWidth<TElement extends HTMLElement>() {
  const [element, setElement] = React.useState<TElement | null>(null);
  const [width, setWidth] = React.useState<number | null>(null);
  const ref = React.useCallback((node: TElement | null) => {
    setElement(node);
  }, []);

  React.useLayoutEffect(() => {
    if (typeof window === "undefined") {
      return;
    }

    if (!element) {
      setWidth(null);
      return;
    }

    const updateWidth = () => {
      const nextWidth = Math.round(element.getBoundingClientRect().width);
      setWidth((current) => (current === nextWidth ? current : nextWidth));
    };

    updateWidth();
    if (typeof ResizeObserver === "undefined") {
      window.addEventListener("resize", updateWidth);
      return () => window.removeEventListener("resize", updateWidth);
    }

    const observer = new ResizeObserver(updateWidth);
    observer.observe(element);
    return () => observer.disconnect();
  }, [element]);

  return [ref, width] as const;
}

function formatTitleYear(title: TitleRecord): string | null {
  if (typeof title.year === "number" && Number.isFinite(title.year)) {
    return String(title.year);
  }

  if (!title.firstAired) {
    return null;
  }
  const parsed = new Date(title.firstAired);
  return Number.isNaN(parsed.getTime()) ? null : String(parsed.getFullYear());
}

function formatRuntimeLabel(minutes: number | null | undefined): string | null {
  if (!minutes || minutes <= 0) {
    return null;
  }

  const hours = Math.floor(minutes / 60);
  const remainingMinutes = minutes % 60;
  if (hours <= 0) {
    return `${remainingMinutes}m`;
  }
  if (remainingMinutes <= 0) {
    return `${hours}h`;
  }
  return `${hours}h ${remainingMinutes}m`;
}

function titleExternalIdValue(
  title: TitleRecord,
  source: string,
): string | null {
  const normalizedSource = source.toLowerCase();
  const idFromList = title.externalIds?.find(
    (entry) => entry.source.toLowerCase() === normalizedSource,
  )?.value;
  const value =
    normalizedSource === "imdb" ? title.imdbId || idFromList : idFromList;
  const trimmed = value?.trim();
  return trimmed || null;
}

function TitleContextSection({
  icon: Icon,
  title,
  description,
  summary,
  action,
  children,
  className,
  collapsible = false,
  defaultOpen = true,
  resetKey,
}: {
  icon: React.ComponentType<{ className?: string }>;
  title: string;
  description?: React.ReactNode;
  summary?: React.ReactNode;
  action?: React.ReactNode;
  children: React.ReactNode;
  className?: string;
  collapsible?: boolean;
  defaultOpen?: boolean;
  resetKey?: React.Key;
}) {
  const [open, setOpen] = React.useState(defaultOpen);

  React.useEffect(() => {
    setOpen(defaultOpen);
  }, [defaultOpen, resetKey]);

  if (collapsible) {
    return (
      <Collapsible open={open} onOpenChange={setOpen}>
        <section
          className={cn(
            "overflow-hidden rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-card2)] shadow-[0_10px_24px_rgba(0,0,0,0.16)]",
            className,
          )}
        >
          <div className="flex min-w-0 items-stretch">
            <CollapsibleTrigger asChild>
              <button
                type="button"
                className="flex min-w-0 flex-1 items-center gap-3 px-4 py-3.5 text-left transition hover:bg-[var(--scry-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-[var(--scry-focus)]"
              >
                <span className="flex size-7 shrink-0 items-center justify-center rounded-[8px] border border-[var(--scry-line3)] bg-[var(--scry-inset)] text-[var(--scry-accent-text)]">
                  <Icon className="h-3.5 w-3.5" />
                </span>
                <span className="min-w-0 flex-1">
                  <span className="block text-[14px] font-semibold tracking-normal text-[var(--scry-ink2)]">
                    {title}
                  </span>
                  {description ? (
                    <span className="mt-1 block text-[11.5px] leading-5 text-[var(--scry-muted3)]">
                      {description}
                    </span>
                  ) : null}
                </span>
                {summary ? (
                  <span className="max-w-[8rem] shrink-0 truncate text-right text-[11.5px] font-semibold text-[var(--scry-muted2)]">
                    {summary}
                  </span>
                ) : null}
                <ChevronDown
                  className={cn(
                    "h-4 w-4 shrink-0 text-[var(--scry-muted3)] transition-transform",
                    open && "rotate-180",
                  )}
                />
              </button>
            </CollapsibleTrigger>
            {action ? (
              <div className="flex shrink-0 items-center py-3.5 pr-4">
                {action}
              </div>
            ) : null}
          </div>
          <CollapsibleContent className="border-t border-[var(--scry-line3)] p-4">
            {children}
          </CollapsibleContent>
        </section>
      </Collapsible>
    );
  }

  return (
    <section
      className={cn(
        "rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-card2)] p-4 shadow-[0_10px_24px_rgba(0,0,0,0.16)]",
        className,
      )}
    >
      <div className="mb-3 flex min-w-0 items-start justify-between gap-3">
        <div className="flex min-w-0 items-start gap-2.5">
          <span className="mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-[8px] border border-[var(--scry-line3)] bg-[var(--scry-inset)] text-[var(--scry-accent-text)]">
            <Icon className="h-3.5 w-3.5" />
          </span>
          <div className="min-w-0">
            <h3 className="text-[14px] font-semibold tracking-normal text-[var(--scry-ink2)]">
              {title}
            </h3>
            {description ? (
              <p className="mt-1 text-[11.5px] leading-5 text-[var(--scry-muted3)]">
                {description}
              </p>
            ) : null}
          </div>
        </div>
        {summary ? (
          <div className="shrink-0 text-[11.5px] font-semibold text-[var(--scry-muted2)]">
            {summary}
          </div>
        ) : null}
        {action ? <div className="shrink-0">{action}</div> : null}
      </div>
      {children}
    </section>
  );
}

type TitleContextRecommendation = {
  item: DiscoveryItem;
  matchPercent: number;
  reason: string;
};

type TitleContextRecommendationGroup = {
  id: string;
  label: string;
  recommendations: TitleContextRecommendation[];
};

function buildTitleContextRecommendationGroups(
  items: DiscoveryItem[],
  t: Translate,
): TitleContextRecommendationGroup[] {
  if (items.length === 0) {
    return [];
  }

  const rankedEntries = items
    .filter((item) => !item.ownedInInput)
    .map((item) => ({
      item,
      score:
        (item.rankScore ?? 0) +
        (item.sourceCount ?? 0) +
        (item.relationCount ?? 0) +
        (item.matchedSubjectCount ?? 0),
    }))
    .sort((left, right) =>
      right.score === left.score
        ? left.item.displayTitle.localeCompare(right.item.displayTitle)
        : right.score - left.score,
    );
  if (rankedEntries.length === 0) {
    return [];
  }
  const maxScore = Math.max(1, rankedEntries[0]?.score ?? 1);
  const toRecommendation = (
    entry: (typeof rankedEntries)[number],
    reason: string,
  ): TitleContextRecommendation => ({
    item: entry.item,
    matchPercent: Math.round(78 + (entry.score / maxScore) * 20),
    reason,
  });
  const groups: TitleContextRecommendationGroup[] = [
    {
      id: "top",
      label: t("title.contextForYouTop"),
      recommendations: rankedEntries
        .slice(0, 4)
        .map((entry) =>
          toRecommendation(entry, t("title.contextForYouReasonTop")),
        ),
    },
  ];

  const genreCounts = new Map<string, { label: string; count: number }>();
  for (const item of items) {
    for (const genre of item.genres ?? []) {
      const label = genre.trim();
      if (!label) {
        continue;
      }
      const key = label.toLocaleLowerCase();
      const current = genreCounts.get(key);
      genreCounts.set(key, {
        label: current?.label ?? label,
        count: (current?.count ?? 0) + 1,
      });
    }
  }

  const topGenre = [...genreCounts.values()].sort(
    (a, b) => b.count - a.count || a.label.localeCompare(b.label),
  )[0];
  if (topGenre) {
    const genreRecommendations = rankedEntries
      .filter((entry) =>
        entry.item.genres?.some(
          (genre) =>
            genre.trim().toLocaleLowerCase() ===
            topGenre.label.toLocaleLowerCase(),
        ),
      )
      .slice(0, 4)
      .map((entry) =>
        toRecommendation(
          entry,
          t("title.contextForYouReasonGenre", { genre: topGenre.label }),
        ),
      );

    if (genreRecommendations.length > 0) {
      groups.push({
        id: "genre",
        label: t("title.contextForYouGenre", { genre: topGenre.label }),
        recommendations: genreRecommendations,
      });
    }
  }

  return groups;
}

function TitleContextRecommendationButton({
  recommendation,
  view,
  t,
  canManageTitle,
  canRequestMedia,
  onAction,
}: {
  recommendation: TitleContextRecommendation;
  view: ViewId;
  t: Translate;
  canManageTitle: boolean;
  canRequestMedia: boolean;
  onAction: (item: DiscoveryItem) => void;
}) {
  const item = recommendation.item;
  const posterUrl = selectPosterVariantUrl(item.posterUrl, "w70");
  const yearLabel =
    typeof item.year === "number" && Number.isFinite(item.year)
      ? String(item.year)
      : null;
  const matchLabel = t("title.contextForYouMatch", {
    match: `${recommendation.matchPercent}%`,
  });
  const owned = item.ownedInInput;
  const ActionIcon = owned ? Check : canManageTitle ? Plus : Send;
  const actionLabel = owned
    ? t("discovery.inLibrary")
    : canManageTitle
      ? t("discovery.add")
      : t("discovery.request");
  const actionDisabled = owned || (!canManageTitle && !canRequestMedia);

  return (
    <div className="group flex min-w-0 items-center gap-3 rounded-[11px] border border-[var(--scry-border)] bg-[var(--scry-card2)] p-2 transition hover:border-[var(--scry-bhover)] hover:bg-[var(--scry-hover)]">
      <div className="flex min-w-0 flex-1 gap-3 text-left">
        <div className="h-[62px] w-[42px] shrink-0 overflow-hidden rounded-[6px] border border-[var(--scry-border2)] bg-[var(--scry-soft)]">
          <TitlePosterSlot
            src={posterUrl}
            alt={t("media.posterAlt", { name: item.displayTitle })}
            className="h-full w-full object-cover"
            placeholderClassName="flex h-full w-full items-center justify-center px-1 text-center text-[9px] text-[var(--scry-muted3)]"
            emptyLabel={t("label.noArt")}
            loading="lazy"
            decoding="async"
          />
        </div>
        <span className="min-w-0 flex-1">
          <span className="block truncate text-[13px] font-semibold text-[var(--scry-ink2)]">
            {item.displayTitle}
          </span>
          <span className="mt-1 block truncate text-[11px] text-[var(--scry-faint)]">
            {[yearLabel, matchLabel].filter(Boolean).join(" / ") ||
              mediaTitleLabel(view, t)}
          </span>
          <span className="mt-1 block truncate text-[11px] text-[var(--scry-muted3)]">
            {recommendation.reason}
          </span>
        </span>
      </div>
      <Button
        type="button"
        variant="outline"
        size="sm"
        className="h-8 w-8 shrink-0 rounded-[8px] border-transparent bg-[rgba(var(--scry-accent-rgb),0.16)] p-0 text-[var(--scry-accent-text)] shadow-none hover:bg-[rgba(var(--scry-accent-rgb),0.24)]"
        aria-label={`${actionLabel}: ${item.displayTitle}`}
        disabled={actionDisabled}
        onClick={() => onAction(item)}
      >
        <ActionIcon className="h-3.5 w-3.5" />
      </Button>
    </div>
  );
}

function titleNormalizedGenreSet(title: TitleRecord): Set<string> {
  return new Set(
    (title.genres ?? [])
      .map((genre) => genre.trim().toLocaleLowerCase())
      .filter(Boolean),
  );
}

function titleSharedGenreCount(
  leftGenres: ReadonlySet<string>,
  right: DiscoveryItem,
): number {
  if (leftGenres.size === 0) {
    return 0;
  }

  let shared = 0;
  for (const genre of right.genres ?? []) {
    if (leftGenres.has(genre.trim().toLocaleLowerCase())) {
      shared += 1;
    }
  }
  return shared;
}

function discoveryItemMatchesView(item: DiscoveryItem, view: ViewId): boolean {
  const raw = `${item.targetKind} ${item.contentType ?? ""} ${item.facetTerms.join(" ")}`.toLowerCase();
  if (view === "anime") {
    return raw.includes("anime");
  }
  if (view === "series") {
    return raw.includes("series") || raw.includes("show");
  }
  return raw.includes("movie") || raw.includes("film");
}

function buildTitleMoreLikeThisDiscoveryItems(
  title: TitleRecord,
  items: DiscoveryItem[],
  view: ViewId,
): DiscoveryItem[] {
  const titleGenres = titleNormalizedGenreSet(title);
  return items
    .filter(
      (candidate) =>
        !candidate.ownedInInput && discoveryItemMatchesView(candidate, view),
    )
    .map((candidate) => {
      const sharedGenres = titleSharedGenreCount(titleGenres, candidate);
      return {
        candidate,
        score:
          sharedGenres * 8 +
          (candidate.rankScore ?? 0) +
          (candidate.relationCount ?? 0) +
          (candidate.matchedSubjectCount ?? 0),
      };
    })
    .sort((left, right) => {
      const scoreDelta = right.score - left.score;
      return scoreDelta !== 0
        ? scoreDelta
        : left.candidate.displayTitle.localeCompare(right.candidate.displayTitle);
    })
    .slice(0, 5)
    .map(({ candidate }) => candidate);
}

function TitleContextActionButton({
  icon: Icon,
  label,
  loading = false,
  destructive = false,
  active = false,
  disabled = false,
  expanded,
  controlsId,
  onClick,
}: {
  icon: React.ComponentType<{ className?: string }>;
  label: string;
  loading?: boolean;
  destructive?: boolean;
  active?: boolean;
  disabled?: boolean;
  expanded?: boolean;
  controlsId?: string;
  onClick: () => void;
}) {
  const actionDisabled = disabled || loading;

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span
          className="inline-flex shrink-0"
          tabIndex={actionDisabled ? 0 : undefined}
        >
          <button
            type="button"
            aria-label={label}
            aria-expanded={expanded}
            aria-controls={controlsId}
            className={cn(
              "flex size-10 shrink-0 items-center justify-center rounded-[10px] border border-[var(--scry-border2)] bg-[var(--scry-card)] text-[var(--scry-muted3)] transition hover:border-[var(--scry-bhover2)] hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-focus)] disabled:cursor-not-allowed disabled:opacity-55",
              active
                ? "border-[rgba(var(--scry-accent-rgb),0.32)] bg-[rgba(var(--scry-accent-rgb),0.12)] text-[var(--scry-accent-text)]"
                : destructive
                  ? "border-destructive/25 text-destructive hover:text-destructive"
                  : "",
            )}
            disabled={actionDisabled}
            onClick={onClick}
            tabIndex={actionDisabled ? -1 : undefined}
          >
            {loading ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <Icon className="h-4 w-4" />
            )}
          </button>
        </span>
      </TooltipTrigger>
      <TooltipContent side="bottom" align="center">
        {label}
      </TooltipContent>
    </Tooltip>
  );
}

function TitleContextMoreLikeThisStrip({
  items,
  view,
  canManageTitle,
  canRequestMedia,
  onAction,
}: {
  items: DiscoveryItem[];
  view: ViewId;
  canManageTitle: boolean;
  canRequestMedia: boolean;
  onAction: (item: DiscoveryItem) => void;
}) {
  const t = useTranslate();

  if (items.length === 0) {
    return null;
  }

  return (
    <TitleContextSection
      icon={Sparkles}
      title={t("title.contextMoreLikeThis")}
      className="bg-[var(--scry-surf)]"
    >
      <div className="flex gap-3 overflow-x-auto pb-2">
        {items.map((item) => {
          const posterUrl = selectPosterVariantUrl(item.posterUrl, "w250");
          const yearLabel =
            typeof item.year === "number" && Number.isFinite(item.year)
              ? String(item.year)
              : null;
          const genreLabel =
            item.genres?.find((candidate) => candidate.trim().length > 0) ??
            null;
          const owned = item.ownedInInput;
          const ActionIcon = owned ? Check : canManageTitle ? Plus : Send;
          const actionLabel = owned
            ? t("discovery.inLibrary")
            : canManageTitle
              ? t("discovery.add")
              : t("discovery.request");
          const actionDisabled = owned || (!canManageTitle && !canRequestMedia);

          return (
            <div key={item.id} className="group w-24 shrink-0 text-left">
              <div className="relative h-[142px] w-24 overflow-hidden rounded-[9px] border border-[var(--scry-border2)] bg-[var(--scry-soft)] shadow-[0_6px_16px_rgba(0,0,0,0.28)] transition group-hover:border-[var(--scry-bhover2)] group-hover:shadow-[0_10px_22px_rgba(0,0,0,0.36)]">
                <TitlePosterSlot
                  src={posterUrl}
                  alt={t("media.posterAlt", { name: item.displayTitle })}
                  className="h-full w-full object-cover"
                  placeholderClassName="flex h-full w-full items-center justify-center px-2 text-center text-[10px] text-[var(--scry-muted3)]"
                  emptyLabel={t("label.noArt")}
                  loading="lazy"
                  decoding="async"
                />
                <div className="pointer-events-none absolute inset-0 bg-[linear-gradient(180deg,transparent_55%,rgba(4,6,12,0.82))]" />
                <button
                  type="button"
                  className="absolute right-1.5 top-1.5 flex size-6 items-center justify-center rounded-[7px] border border-white/15 bg-slate-950/70 text-[var(--scry-text2)] backdrop-blur-sm transition hover:bg-[var(--scry-accent)] hover:text-white disabled:cursor-default disabled:opacity-60"
                  aria-label={`${actionLabel}: ${item.displayTitle}`}
                  disabled={actionDisabled}
                  onClick={() => onAction(item)}
                >
                  <ActionIcon className="h-3.5 w-3.5" />
                </button>
              </div>
              <p className="mt-2 truncate text-[12px] font-semibold text-[var(--scry-body)]">
                {item.displayTitle}
              </p>
              <p className="truncate text-[11px] text-[var(--scry-faint)]">
                {[yearLabel, genreLabel].filter(Boolean).join(" / ") ||
                  mediaTitleLabel(view, t)}
              </p>
            </div>
          );
        })}
      </div>
    </TitleContextSection>
  );
}

function TitleContextForYouPanel({
  discoveryItems,
  view,
  canManageTitle,
  canRequestMedia,
  onDiscoveryAction,
}: {
  discoveryItems: DiscoveryItem[];
  view: ViewId;
  canManageTitle: boolean;
  canRequestMedia: boolean;
  onDiscoveryAction: (item: DiscoveryItem) => void;
}) {
  const t = useTranslate();
  const recommendationGroups = React.useMemo(
    () => buildTitleContextRecommendationGroups(discoveryItems, t),
    [discoveryItems, t],
  );

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-y-auto p-[18px]">
      <div className="flex items-center gap-3">
        <div className="flex size-9 shrink-0 items-center justify-center rounded-[10px] border border-[var(--scry-baccent)] bg-[linear-gradient(135deg,rgba(var(--scry-accent-rgb),0.32),rgba(155,91,255,0.2))] text-[var(--scry-accent-text)]">
          <Sparkles className="h-[18px] w-[18px]" />
        </div>
        <div className="min-w-0">
          <p className="text-[16px] font-semibold text-[var(--scry-ink2)]">
            {t("title.contextForYouTitle")}
          </p>
          <p className="mt-0.5 text-[11.5px] text-[var(--scry-muted3)]">
            {t("title.contextForYouSubtitle")}
          </p>
        </div>
      </div>
      <p className="mx-0.5 mt-3 text-[12px] leading-5 text-[var(--scry-faint)]">
        {t("title.contextForYouBody")}
      </p>

      {recommendationGroups.length === 0 ? (
        <div className="flex min-h-[16rem] flex-1 flex-col items-center justify-center gap-3 px-4 text-center">
          <div className="flex size-12 items-center justify-center rounded-[14px] border border-[var(--scry-border2)] bg-[var(--scry-inset)] text-[var(--scry-muted2)]">
            <LayoutList className="h-5 w-5" />
          </div>
          <div>
            <p className="text-sm font-semibold text-[var(--scry-ink2)]">
              {t("title.contextForYouEmptyTitle")}
            </p>
            <p className="mt-1 text-[12px] leading-5 text-[var(--scry-muted3)]">
              {t("title.contextForYouEmptyBody")}
            </p>
          </div>
        </div>
      ) : (
        <div className="mt-4 space-y-5">
          {recommendationGroups.map((group) => (
            <section key={group.id}>
              <h3 className="mx-0.5 mb-2.5 text-[11px] font-bold uppercase tracking-[0.06em] text-[var(--scry-faint2)]">
                {group.label}
              </h3>
              <div className="grid grid-cols-[repeat(auto-fill,minmax(min(248px,100%),1fr))] gap-2.5">
                {group.recommendations.map((recommendation) => (
                  <TitleContextRecommendationButton
                    key={`${group.id}-${recommendation.item.id}`}
                    recommendation={recommendation}
                    view={view}
                    t={t}
                    canManageTitle={canManageTitle}
                    canRequestMedia={canRequestMedia}
                    onAction={onDiscoveryAction}
                  />
                ))}
              </div>
            </section>
          ))}
        </div>
      )}
    </div>
  );
}

function TitleContextReleaseSearchPanel({
  title,
  onInteractiveSearch,
  onQueueFromInteractive,
  onQueueAdditionalFromInteractive,
  disabled = false,
  runRequestId = 0,
  onLoadingChange,
}: {
  title: TitleRecord;
  onInteractiveSearch: (title: TitleRecord) => Promise<Release[]> | Release[];
  onQueueFromInteractive: (
    title: TitleRecord,
    release: Release,
  ) => Promise<void> | void;
  onQueueAdditionalFromInteractive: (
    title: TitleRecord,
    release: Release,
  ) => Promise<void> | void;
  disabled?: boolean;
  runRequestId?: number;
  onLoadingChange?: (loading: boolean) => void;
}) {
  const t = useTranslate();
  const requestIdRef = React.useRef(0);
  const lastRunRequestIdRef = React.useRef(0);
  const [results, setResults] = React.useState<Release[] | null>(null);
  const [loading, setLoading] = React.useState(false);
  const [searchFailed, setSearchFailed] = React.useState(false);
  const [sortKey, setSortKey] =
    React.useState<ReleaseSearchSortKey>("score");
  const [sortDirection, setSortDirection] =
    React.useState<ReleaseSearchSortDirection>("desc");
  const releaseSearchDescription = React.useMemo(() => {
    if (results === null) {
      return t("help.interactiveSearchTooltip");
    }

    const sourceCount = new Set(
      results
        .map((release) => release.source?.trim())
        .filter((source): source is string => Boolean(source)),
    ).size;
    return t("title.contextReleaseSearchSummary", {
      releaseCount: results.length,
      indexerCount: sourceCount,
    });
  }, [results, t]);

  const handleSortChange = React.useCallback(
    (
      nextKey: ReleaseSearchSortKey,
      nextDirection: ReleaseSearchSortDirection,
    ) => {
      setSortKey(nextKey);
      setSortDirection(nextDirection);
    },
    [],
  );

  const toggleSort = React.useCallback(
    (nextKey: ReleaseSearchSortKey) => {
      const nextDirection: ReleaseSearchSortDirection =
        sortKey === nextKey && sortDirection === "desc" ? "asc" : "desc";
      handleSortChange(nextKey, nextDirection);
    },
    [handleSortChange, sortDirection, sortKey],
  );

  const renderSortIcon = React.useCallback(
    (key: ReleaseSearchSortKey) => {
      if (sortKey !== key) {
        return <ChevronDown className="h-3 w-3 opacity-45" />;
      }
      return sortDirection === "desc" ? (
        <ArrowDown className="h-3 w-3" />
      ) : (
        <ArrowUp className="h-3 w-3" />
      );
    },
    [sortDirection, sortKey],
  );

  React.useEffect(() => {
    requestIdRef.current += 1;
    setResults(null);
    setLoading(false);
    setSearchFailed(false);
  }, [title.id]);

  React.useEffect(() => {
    onLoadingChange?.(loading);
  }, [loading, onLoadingChange]);

  const runSearch = React.useCallback(() => {
    if (disabled || loading) {
      return;
    }

    const requestId = requestIdRef.current + 1;
    requestIdRef.current = requestId;
    setLoading(true);
    setSearchFailed(false);

    void Promise.resolve(onInteractiveSearch(title))
      .then((nextResults) => {
        if (requestIdRef.current !== requestId) {
          return;
        }
        setResults(nextResults);
      })
      .catch(() => {
        if (requestIdRef.current !== requestId) {
          return;
        }
        setResults([]);
        setSearchFailed(true);
      })
      .finally(() => {
        if (requestIdRef.current === requestId) {
          setLoading(false);
        }
      });
  }, [disabled, loading, onInteractiveSearch, title]);

  React.useEffect(() => {
    if (runRequestId <= 0 || lastRunRequestIdRef.current === runRequestId) {
      return;
    }
    lastRunRequestIdRef.current = runRequestId;
    runSearch();
  }, [runRequestId, runSearch]);

  const showRetrySearchControl =
    results === null || searchFailed || results.length === 0;

  return (
    <section className="overflow-hidden rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-card2)]">
      <div className="flex flex-wrap items-center gap-3 border-b border-[var(--scry-line3)] px-4 py-3.5">
        <span className="flex size-8 shrink-0 items-center justify-center rounded-[9px] border border-[var(--scry-baccent)] bg-[rgba(var(--scry-accent-rgb),0.15)] text-[var(--scry-accent-text)]">
          <Search className="h-4 w-4" />
        </span>
        <div className="min-w-0 flex-1">
          <h3 className="truncate text-[14.5px] font-bold text-[var(--scry-ink2)]">
            {t("label.interactiveSearch")}
          </h3>
          <p className="mt-0.5 truncate text-[11.5px] text-[var(--scry-faint)]">
            {releaseSearchDescription}
          </p>
        </div>
        <div className="flex flex-wrap items-center justify-end gap-1.5">
          {results && results.length > 1 ? (
            <>
              <Button
                type="button"
                size="sm"
                variant={sortKey === "score" ? "secondary" : "outline"}
                className="h-[30px] shrink-0 rounded-[8px] border-[var(--scry-border2)] px-2.5 text-[11px] font-semibold"
                onClick={() => toggleSort("score")}
              >
                <span>Score</span>
                {renderSortIcon("score")}
              </Button>
              <Button
                type="button"
                size="sm"
                variant={sortKey === "size" ? "secondary" : "outline"}
                className="h-[30px] shrink-0 rounded-[8px] border-[var(--scry-border2)] px-2.5 text-[11px] font-semibold"
                onClick={() => toggleSort("size")}
              >
                <span>Size</span>
                {renderSortIcon("size")}
              </Button>
            </>
          ) : null}
          {showRetrySearchControl ? (
            <Button
              type="button"
              size="sm"
              variant={results === null ? "secondary" : "outline"}
              className="h-[30px] shrink-0 rounded-[8px] px-2.5 text-[11px] font-semibold"
              onClick={runSearch}
              disabled={loading || disabled}
            >
              {loading ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <Search className="h-3.5 w-3.5" />
              )}
              <span>{loading ? t("label.searching") : t("label.search")}</span>
            </Button>
          ) : null}
        </div>
      </div>
      <div
        className={cn(
          results !== null && results.length > 0 && !searchFailed ? "" : "p-4",
        )}
      >
        {loading && results === null ? (
          <div className="flex items-center gap-2 rounded-[10px] border border-[var(--scry-border2)] bg-[var(--scry-soft)] px-3 py-2 text-[12px] text-[var(--scry-muted2)]">
            <Loader2 className="h-4 w-4 animate-spin text-[var(--scry-accent)]" />
            {t("title.searchingReleases")}
          </div>
        ) : searchFailed ? (
          <p className="rounded-[10px] border border-rose-500/25 bg-rose-500/10 px-3 py-2 text-[12px] text-rose-700 dark:text-rose-300">
            {t("nzb.searchFailed")}
          </p>
        ) : results === null ? (
          <p className="rounded-[10px] border border-[var(--scry-border2)] bg-[var(--scry-soft)] px-3 py-2 text-[12px] leading-5 text-[var(--scry-muted3)]">
            {t("nzb.noResultsYet")}
          </p>
        ) : results.length === 0 ? (
          <p className="rounded-[10px] border border-[var(--scry-border2)] bg-[var(--scry-soft)] px-3 py-2 text-[12px] leading-5 text-[var(--scry-muted3)]">
            {t("title.noReleasesFound", { name: title.name })}
          </p>
        ) : (
          <SearchResultBuckets
            results={results}
            onQueue={(release) => onQueueFromInteractive(title, release)}
            onQueueAdditional={(release) =>
              onQueueAdditionalFromInteractive(title, release)
            }
            canQueueAdditional={(release) =>
              releaseSupportsAdditionalFileQueue(release, title.facet)
            }
            disabled={disabled}
            requireCandidateToken
            sortKey={sortKey}
            sortDirection={sortDirection}
            onSortChange={handleSortChange}
            hideInlineSortControls
            showBlockedInline
            presentation="selected-title"
          />
        )}
      </div>
    </section>
  );
}

function TitleContextPanel({
  id,
  title,
  titles,
  view,
  overviewTargetView,
  resolvedProfileName,
  blocklistEntries,
  externalSubtitles,
  qualityProfiles,
  qualityProfilesLoading,
  isTogglingMonitored,
  isDeleting,
  onOpenOverview,
  onToggleMonitored,
  onAutoQueue,
  onRefreshTitles,
  onRefreshSubtitles,
  onDeleteMediaFile,
  onMakePrimaryMediaFile,
  primaryMediaFileUpdatingId,
  onPreviewRename,
  onApplyRename,
  refreshLoading,
  onInteractiveSearch,
  onQueueFromInteractive,
  onQueueAdditionalFromInteractive,
  bulkActionBusy,
  onDelete,
  onClearSelection,
  onSelectTitle,
  titleListDisclosure,
  className,
}: {
  id?: string;
  title: TitleRecord | null;
  titles: TitleRecord[];
  view: ViewId;
  overviewTargetView: ViewId;
  resolvedProfileName: string | null;
  blocklistEntries: TitleReleaseBlocklistEntry[];
  externalSubtitles: ExternalSubtitleRecord[];
  qualityProfiles: ParsedQualityProfile[];
  qualityProfilesLoading: boolean;
  isTogglingMonitored: boolean;
  isDeleting: boolean;
  onOpenOverview: (
    targetView: ViewId,
    overviewTarget: OverviewTitleTarget,
  ) => void;
  onToggleMonitored?: (
    title: TitleRecord,
    monitored: boolean,
  ) => Promise<void> | void;
  onAutoQueue: (title: TitleRecord) => Promise<void> | void;
  onRefreshTitles: () => Promise<void> | void;
  onRefreshSubtitles: () => Promise<void> | void;
  onDeleteMediaFile: (title: TitleRecord, fileId: string) => void;
  onMakePrimaryMediaFile: (
    title: TitleRecord,
    fileId: string,
  ) => Promise<void> | void;
  primaryMediaFileUpdatingId: string | null;
  onPreviewRename: (
    title: TitleRecord,
  ) => Promise<MediaRenamePlan | null> | MediaRenamePlan | null;
  onApplyRename: (
    title: TitleRecord,
    plan: MediaRenamePlan,
  ) => Promise<boolean | void> | boolean | void;
  refreshLoading: boolean;
  onInteractiveSearch: (title: TitleRecord) => Promise<Release[]> | Release[];
  onQueueFromInteractive: (
    title: TitleRecord,
    release: Release,
  ) => Promise<void> | void;
  onQueueAdditionalFromInteractive: (
    title: TitleRecord,
    release: Release,
  ) => Promise<void> | void;
  bulkActionBusy: boolean;
  onDelete: (title: TitleRecord) => void;
  onClearSelection: () => void;
  onSelectTitle: (title: TitleRecord) => void;
  titleListDisclosure?: React.ReactNode;
  className?: string;
}) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const [autoQueueLoadingTitleId, setAutoQueueLoadingTitleId] = React.useState<
    string | null
  >(null);
  const [releaseSearchRequestId, setReleaseSearchRequestId] = React.useState(0);
  const [releaseSearchLoading, setReleaseSearchLoading] = React.useState(false);
  const [releaseSearchTitleId, setReleaseSearchTitleId] = React.useState<
    string | null
  >(null);
  const [renamePlan, setRenamePlan] = React.useState<MediaRenamePlan | null>(
    null,
  );
  const [renamePreviewing, setRenamePreviewing] = React.useState(false);
  const [renameApplying, setRenameApplying] = React.useState(false);
  const [historyOpen, setHistoryOpen] = React.useState(false);
  const [blockedReleasesOpen, setBlockedReleasesOpen] =
    React.useState(false);
  const releaseSearchOpen = title !== null && releaseSearchTitleId === title.id;
  const releaseSearchActionLoading = releaseSearchOpen && releaseSearchLoading;
  const panelClassName = cn(
    "min-h-0 w-full min-w-0 flex-col overflow-hidden rounded-[16px] border border-[var(--scry-border2)] bg-[var(--scry-surfD)] shadow-[0_18px_44px_rgba(15,23,42,0.10)]",
    className,
  );
  const moreLikeThisTitles = React.useMemo(
    () => (title ? buildTitleMoreLikeThisTitles(title, titles) : []),
    [title, titles],
  );
  const titleMediaFiles = React.useMemo<MediaFileOnDisk[]>(
    () =>
      (title?.mediaFiles ?? []).flatMap((file) => {
        const filePath = file.filePath?.trim();
        if (!filePath) {
          return [];
        }
        return [
          {
            ...file,
            filePath,
            sizeBytes: file.sizeBytes ?? null,
            scanStatus: file.scanStatus ?? "unknown",
            videoCodec: file.videoCodec ?? file.videoCodecParsed ?? null,
            videoWidth: file.videoWidth ?? null,
            videoHeight: file.videoHeight ?? null,
            videoBitrateKbps: file.videoBitrateKbps ?? null,
            videoBitDepth: file.videoBitDepth ?? null,
            videoHdrFormat: file.videoHdrFormat ?? null,
            videoFrameRate: file.videoFrameRate ?? null,
            videoProfile: file.videoProfile ?? null,
            audioCodec: file.audioCodec ?? file.audioCodecParsed ?? null,
            audioChannels: file.audioChannels ?? null,
            audioBitrateKbps: file.audioBitrateKbps ?? null,
            audioLanguages: file.audioLanguages ?? [],
            audioStreams: (file.audioStreams ?? []).map((stream) => ({
              codec: stream.codec ?? null,
              channels: stream.channels ?? null,
              language: stream.language ?? null,
              bitrateKbps: stream.bitrateKbps ?? null,
            })),
            subtitleLanguages: file.subtitleLanguages ?? [],
            subtitleCodecs: file.subtitleCodecs ?? [],
            subtitleStreams: (file.subtitleStreams ?? []).map((stream) => ({
              codec: stream.codec ?? null,
              language: stream.language ?? null,
              name: stream.name ?? null,
              forced: stream.forced ?? false,
              default: stream.default ?? false,
            })),
            hasMultiaudio: file.hasMultiaudio ?? false,
            durationSeconds: file.durationSeconds ?? null,
            numChapters: file.numChapters ?? null,
            containerFormat: file.containerFormat ?? null,
          },
        ];
      }),
    [title?.mediaFiles],
  );

  React.useEffect(() => {
    setReleaseSearchLoading(false);
    setReleaseSearchTitleId(null);
  }, [title?.id]);
  React.useEffect(() => {
    setRenamePlan(null);
    setRenamePreviewing(false);
    setRenameApplying(false);
    setHistoryOpen(false);
    setBlockedReleasesOpen(false);
  }, [title?.facet, title?.id]);

  const handlePreviewRename = React.useCallback(async () => {
    if (!title) {
      return;
    }

    setRenamePreviewing(true);
    try {
      setRenamePlan(await onPreviewRename(title));
    } finally {
      setRenamePreviewing(false);
    }
  }, [onPreviewRename, title]);

  const handleApplyRename = React.useCallback(async () => {
    if (!title || !renamePlan) {
      return;
    }

    setRenameApplying(true);
    try {
      const applied = await onApplyRename(title, renamePlan);
      if (applied !== false) {
        setRenamePlan(null);
      }
    } finally {
      setRenameApplying(false);
    }
  }, [onApplyRename, renamePlan, title]);

  if (!title) {
    return (
      <aside
        id={id}
        aria-label={t("title.contextPanelTitle")}
        className={panelClassName}
      >
        <TitleContextForYouPanel
          titles={titles}
          view={view}
          onSelectTitle={onSelectTitle}
        />
      </aside>
    );
  }

  const posterUrl = selectPosterVariantUrl(title.posterUrl, "w250");
  const backgroundUrl =
    title.backgroundUrl ?? title.backgroundSourceUrl ?? posterUrl ?? null;
  const yearLabel = formatTitleYear(title);
  const statusLabel = localizedTitleStatus(t, title.contentStatus);
  const addedAtLabel =
    formatTitleDate(title.createdAt, dateTimeFormat) ?? t("label.unknown");
  const unknownLabel = t("label.unknown");
  const qualityLabel = qualityProfilesLoading
    ? t("label.loading")
    : resolveDisplayedQualityLabel(
        title,
        qualityProfiles,
        resolvedProfileName,
        unknownLabel,
      );
  const overviewText =
    title.overview?.trim() || t("title.descriptionUnavailable");
  const subheading = yearLabel ?? mediaTitleLabel(view, t);
  const libraryLabel = title.libraryName ?? title.libraryId;
  const runtimeLabel = formatRuntimeLabel(title.runtimeMinutes);
  const loadingLabel = t("label.loading");
  const studioOrNetworkLabel =
    view === "movies"
      ? title.studio?.trim()
      : title.network?.trim() || title.studio?.trim();
  const imdbId = titleExternalIdValue(title, "imdb");
  const tmdbId = titleExternalIdValue(title, "tmdb");
  const tvdbId = titleExternalIdValue(title, "tvdb");
  const malId = titleExternalIdValue(title, "mal");
  const anilistId = titleExternalIdValue(title, "anilist");
  const anidbId = titleExternalIdValue(title, "anidb");
  const hasExternalLinks = Boolean(
    imdbId || tmdbId || tvdbId || malId || anilistId || anidbId,
  );
  const heroAccentPills = [
    statusLabel,
    qualityLabel === unknownLabel || qualityLabel === loadingLabel
      ? null
      : qualityLabel,
  ].filter((value): value is string => Boolean(value));
  const heroMutedMetadata = [
    runtimeLabel,
    studioOrNetworkLabel,
  ].filter((value): value is string => Boolean(value));
  const heroGenreLabels = (title.genres ?? [])
    .map((genre) => genre.trim())
    .filter(Boolean)
    .slice(0, 4);
  const autoQueueLoading = autoQueueLoadingTitleId === title.id;
  const releaseSearchPanelId = `title-context-release-search-${title.id}`;
  const handleAutoQueue = async () => {
    setAutoQueueLoadingTitleId(title.id);
    try {
      await onAutoQueue(title);
    } finally {
      setAutoQueueLoadingTitleId((current) =>
        current === title.id ? null : current,
      );
    }
  };
  const handleInteractiveSearchAction = () => {
    if (releaseSearchOpen) {
      setReleaseSearchTitleId(null);
      setReleaseSearchLoading(false);
      return;
    }
    setReleaseSearchTitleId(title.id);
    setReleaseSearchRequestId((current) => current + 1);
  };

  return (
    <aside
      id={id}
      aria-label={t("title.contextPanelTitle")}
      className={panelClassName}
    >
      <div
        data-slot="title-context-scroll"
        className="relative min-h-0 flex-1 overflow-y-auto p-4 sm:p-5"
      >
        {titleListDisclosure ? (
          <div className="mb-3 flex items-center">{titleListDisclosure}</div>
        ) : null}
        <section className="relative overflow-hidden rounded-[14px] border border-[var(--scry-border2)] bg-[linear-gradient(135deg,var(--scry-surfE),var(--scry-bg))] shadow-[0_18px_44px_rgba(0,0,0,0.28)]">
          {backgroundUrl ? (
            <img
              src={backgroundUrl}
              alt=""
              aria-hidden="true"
              className="absolute inset-0 h-full w-full object-cover opacity-40 saturate-90"
              loading="lazy"
            />
          ) : null}
          <div className="absolute inset-0 bg-[linear-gradient(105deg,rgba(8,12,22,0.96)_28%,rgba(8,12,22,0.72)_68%,rgba(8,12,22,0.38))] dark:bg-[linear-gradient(105deg,rgba(8,12,22,0.97)_28%,rgba(8,12,22,0.66)_68%,rgba(8,12,22,0.34))]" />
          <button
            type="button"
            aria-label={t("label.clear")}
            className="absolute right-3 top-3 z-10 flex size-8 items-center justify-center rounded-[9px] border border-white/15 bg-slate-950/60 text-[#dde4f5] shadow-sm backdrop-blur-md transition hover:bg-slate-950/75 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-focus)]"
            onClick={onClearSelection}
          >
            <X className="h-4 w-4" />
          </button>
          <div className="relative flex gap-4 p-4 pr-14 sm:gap-5 sm:p-5 sm:pr-16">
            <div className="relative h-44 w-[116px] shrink-0 overflow-hidden rounded-[10px] border border-[#2a3556] bg-[var(--scry-inset)] shadow-[0_12px_32px_rgba(0,0,0,0.5)] sm:h-[198px] sm:w-[132px]">
              <TitlePosterSlot
                src={posterUrl}
                sourceSrc={title.posterSourceUrl}
                metadataFetchedAt={title.metadataFetchedAt}
                createdAt={title.createdAt}
                alt={t("media.posterAlt", { name: title.name })}
                className="h-full w-full object-cover"
                placeholderClassName="flex h-full w-full items-center justify-center px-2 text-center text-[11px] text-[var(--scry-muted3)]"
                emptyLabel={t("label.noArt")}
                loading="lazy"
                decoding="async"
              />
              <div
                className="pointer-events-none absolute inset-0 bg-[linear-gradient(180deg,transparent_42%,rgba(4,6,12,0.86))]"
                aria-hidden="true"
              />
              <span className="pointer-events-none absolute inset-x-2 bottom-2 line-clamp-2 text-[12px] font-bold leading-[1.08] text-white shadow-black [text-shadow:0_1px_6px_rgba(0,0,0,0.75)]">
                {title.name}
              </span>
            </div>
            <div className="min-w-0 flex-1">
              <h2 className="text-[21px] font-bold leading-[1.1] tracking-normal text-white">
                {title.name}
                {yearLabel ? (
                  <span className="font-medium text-[var(--scry-muted3)]">
                    {" "}
                    ({yearLabel})
                  </span>
                ) : null}
              </h2>
              <div className="mt-2 flex flex-wrap items-center gap-2">
                <span
                  className={cn(
                    "inline-flex h-6 items-center gap-1 rounded-[7px] px-2.5 text-[11px] font-semibold",
                    title.monitored
                      ? "bg-emerald-500/15 text-emerald-200"
                      : "bg-white/[0.06] text-[var(--scry-muted2)]",
                  )}
                >
                  {title.monitored ? (
                    <Eye className="h-3.5 w-3.5" />
                  ) : (
                    <EyeOff className="h-3.5 w-3.5" />
                  )}
                  {title.monitored
                    ? t("title.monitored")
                    : t("title.unmonitored")}
                </span>
                {heroAccentPills.map((pill, index) => (
                  <span
                    key={`${index}-${pill}`}
                    className="inline-flex h-6 max-w-[12rem] items-center truncate rounded-[7px] bg-[rgba(var(--scry-accent-rgb),0.15)] px-2.5 text-[11px] font-semibold text-[var(--scry-accent-text)]"
                  >
                    {pill}
                  </span>
                ))}
                {heroMutedMetadata.map((pill, index) => (
                  <span
                    key={`${index}-${pill}`}
                    className="inline-flex h-6 max-w-[12rem] items-center truncate text-[12px] font-medium text-[var(--scry-muted2)]"
                  >
                    {pill}
                  </span>
                ))}
              </div>
              {heroGenreLabels.length > 0 ? (
                <div className="mt-3 flex flex-wrap gap-1.5">
                  {heroGenreLabels.map((genre) => (
                    <span
                      key={genre}
                      className="inline-flex h-6 max-w-[9.5rem] items-center rounded-[7px] border border-white/10 bg-white/[0.06] px-2.5 text-[11px] font-semibold text-[#cfd7ee]"
                    >
                      <span className="min-w-0 truncate">{genre}</span>
                    </span>
                  ))}
                </div>
              ) : null}
              <p className="mt-3 line-clamp-5 text-[12.5px] leading-5 text-[#b7c0dd]">
                {overviewText}
              </p>
              {hasExternalLinks ? (
                <div className="mt-3 flex flex-wrap gap-2 [&_a]:h-8 [&_a]:rounded-[8px] [&_a]:border-white/10 [&_a]:bg-white/[0.07] [&_a]:px-2.5 [&_a]:py-1 [&_a]:text-[11px] [&_a]:text-[#dbe4fb] [&_a:hover]:bg-white/[0.12] [&_img]:h-4 [&_img]:w-4 [&_span]:text-[#dbe4fb]">
                  <ImdbExternalLink imdbId={imdbId} />
                  {view === "movies" ? (
                    <TvdbMovieExternalLink tvdbId={tvdbId} slug={title.slug} />
                  ) : (
                    <TvdbSeriesExternalLink tvdbId={tvdbId} slug={title.slug} />
                  )}
                  <TmdbExternalLink
                    mediaType={view === "movies" ? "movie" : "tv"}
                    tmdbId={tmdbId}
                  />
                  <MalExternalLink malId={malId} />
                  <AnilistExternalLink anilistId={anilistId} />
                  <AnidbExternalLink anidbId={anidbId} />
                </div>
              ) : null}
              <div className="mt-3 flex flex-wrap items-center gap-2 text-[11px] text-[var(--scry-faint2)]">
                <span>{subheading || mediaTitleLabel(view, t)}</span>
                <span aria-hidden="true">/</span>
                <span>
                  {t("title.contextLibrary")}: {libraryLabel}
                </span>
                <span aria-hidden="true">/</span>
                <span>
                  {t("title.contextAdded")}: {addedAtLabel}
                </span>
              </div>
            </div>
          </div>
        </section>

        <TooltipProvider>
          <div className="mt-3 overflow-x-auto">
            <div className="flex gap-2">
              <TitleContextActionButton
                icon={title.monitored ? EyeOff : Eye}
                label={
                  title.monitored
                    ? t("title.unmonitorAction")
                    : t("title.monitorAction")
                }
                active={title.monitored}
                loading={isTogglingMonitored}
                disabled={bulkActionBusy || !onToggleMonitored}
                onClick={() =>
                  void onToggleMonitored?.(title, !title.monitored)
                }
              />
              <TitleContextActionButton
                icon={Zap}
                label={t("title.queueLatest")}
                loading={autoQueueLoading}
                disabled={bulkActionBusy}
                onClick={() => void handleAutoQueue()}
              />
              <TitleContextActionButton
                icon={Search}
                label={t("label.interactiveSearch")}
                active={releaseSearchOpen}
                loading={releaseSearchActionLoading}
                disabled={bulkActionBusy && !releaseSearchOpen}
                expanded={releaseSearchOpen}
                controlsId={releaseSearchPanelId}
                onClick={handleInteractiveSearchAction}
              />
              <TitleContextActionButton
                icon={RefreshCw}
                label={t("label.refresh")}
                loading={refreshLoading}
                disabled={bulkActionBusy || refreshLoading}
                onClick={() => void onRefreshTitles()}
              />
              <TitleContextActionButton
                icon={ClipboardList}
                label={t("activity.history")}
                disabled={bulkActionBusy}
                onClick={() => setHistoryOpen(true)}
              />
              <TitleContextActionButton
                icon={Edit}
                label={t("label.edit")}
                disabled={bulkActionBusy}
                onClick={() => onOpenOverview(overviewTargetView, title)}
              />
              <TitleContextActionButton
                icon={Trash2}
                label={t("label.delete")}
                destructive
                loading={isDeleting}
                disabled={bulkActionBusy}
                onClick={() => onDelete(title)}
              />
            </div>
          </div>
        </TooltipProvider>

        {releaseSearchOpen ? (
          <div
            id={releaseSearchPanelId}
            role="region"
            aria-label={t("label.interactiveSearch")}
            aria-live="polite"
            className="mt-3"
          >
            <TitleContextReleaseSearchPanel
              title={title}
              onInteractiveSearch={onInteractiveSearch}
              onQueueFromInteractive={onQueueFromInteractive}
              onQueueAdditionalFromInteractive={onQueueAdditionalFromInteractive}
              disabled={bulkActionBusy}
              runRequestId={releaseSearchRequestId}
              onLoadingChange={setReleaseSearchLoading}
            />
          </div>
        ) : null}

        <div className="mt-3 space-y-3">
          <section className="rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-card2)] p-4">
            <div className="mb-3.5 flex min-w-0 items-center justify-between gap-3">
              <div className="flex min-w-0 items-center gap-2.5">
                <FolderOpen className="h-4 w-4 shrink-0 text-[var(--scry-accent-text)]" />
                <div className="min-w-0">
                  <h3 className="truncate text-[14px] font-semibold text-[var(--scry-ink2)]">
                    {t("title.filesOnDisk")}
                  </h3>
                </div>
              </div>
              <Button
                type="button"
                variant="default"
                size="sm"
                className="h-[30px] shrink-0 rounded-[8px] border-0 bg-[var(--scry-accent-grad)] px-3 text-[11.5px] font-semibold text-white shadow-none hover:bg-[var(--scry-accent-grad)]"
                onClick={() => {
                  void handlePreviewRename();
                }}
                disabled={renamePreviewing || renameApplying}
              >
                <span>
                  {renamePreviewing
                    ? t("rename.previewing")
                    : t("rename.previewButton")}
                </span>
              </Button>
            </div>
            <div className="border-t border-[var(--scry-line3)] pt-3">
              <MediaFilesOnDiskPanel
                emptyMessage={t("title.noFilesTracked")}
                emptyHint={t("title.noFilesTrackedHint")}
                mediaFiles={titleMediaFiles}
                subtitleDownloads={externalSubtitles}
                onRefreshSubtitles={onRefreshSubtitles}
                onDeleteFile={(fileId) => onDeleteMediaFile(title, fileId)}
                onMakePrimaryFile={
                  title.facet === "movie"
                    ? (fileId) => onMakePrimaryMediaFile(title, fileId)
                    : undefined
                }
                primaryFileUpdatingId={primaryMediaFileUpdatingId}
                showPrimaryRoleBadge
                fileRowIdPrefix={`title-context-file-row-${title.id}`}
                filePathIdPrefix={`title-context-file-path-${title.id}`}
                roleIdPrefix={`title-context-file-role-${title.id}`}
                subtitleSearchIdPrefix={`title-context-file-search-subtitles-${title.id}`}
                deleteFileIdPrefix={`title-context-file-delete-${title.id}`}
                makePrimaryFileIdPrefix={`title-context-file-make-primary-${title.id}`}
                presentation="selected-title"
              />
            </div>
            {renamePlan ? (
              <MediaRenamePlanPanel
                plan={renamePlan}
                applying={renameApplying}
                applyDisabled={renameApplying || renamePlan.renamable === 0}
                applyButtonId={`title-context-rename-apply-${title.id}`}
                onApply={() => {
                  void handleApplyRename();
                }}
              />
            ) : null}
          </section>

          <TitleContextMoreLikeThisStrip
            titles={moreLikeThisTitles}
            view={view}
            onSelectTitle={onSelectTitle}
          />

          {blocklistEntries.length === 0 ? (
            <section className="flex min-h-[3.25rem] items-center gap-2.5 rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-card2)] px-4">
              <ChevronRight className="h-4 w-4 shrink-0 text-[var(--scry-faint)]" />
              <span className="min-w-0 flex-1 truncate text-[13.5px] font-semibold text-[var(--scry-text2)]">
                {t("title.contextBlockedReleases")}
              </span>
              <span className="shrink-0 rounded-[7px] bg-white/[0.06] px-2 py-0.5 text-[11px] font-semibold text-[var(--scry-muted)]">
                {blocklistEntries.length}
              </span>
            </section>
          ) : (
            <Collapsible
              open={blockedReleasesOpen}
              onOpenChange={setBlockedReleasesOpen}
            >
              <section className="overflow-hidden rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-card2)]">
                <CollapsibleTrigger asChild>
                  <button
                    type="button"
                    className="flex min-h-[3.25rem] w-full min-w-0 items-center gap-2.5 px-4 text-left transition hover:bg-[var(--scry-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-[var(--scry-focus)]"
                  >
                    <ChevronRight
                      className={cn(
                        "h-4 w-4 shrink-0 text-[var(--scry-faint)] transition-transform",
                        blockedReleasesOpen && "rotate-90",
                      )}
                    />
                    <span className="min-w-0 flex-1 truncate text-[13.5px] font-semibold text-[var(--scry-text2)]">
                      {t("title.contextBlockedReleases")}
                    </span>
                    <span className="shrink-0 rounded-[7px] bg-white/[0.06] px-2 py-0.5 text-[11px] font-semibold text-[var(--scry-muted)]">
                      {blocklistEntries.length}
                    </span>
                  </button>
                </CollapsibleTrigger>
                <CollapsibleContent className="border-t border-[var(--scry-line3)] p-4">
                  <div className="space-y-2">
                    {blocklistEntries.map((entry) => {
                      const attemptedAtLabel = formatTitleDate(
                        entry.attemptedAt,
                        dateTimeFormat,
                      );
                      const releaseLabel =
                        entry.sourceTitle?.trim() ||
                        entry.sourceHint?.trim() ||
                        t("episode.untitledRelease");

                      return (
                        <div
                          key={entry.id}
                          className="rounded-[11px] border border-[var(--scry-line3)] bg-[var(--scry-inset)] p-3"
                        >
                          <div className="flex min-w-0 items-start justify-between gap-3">
                            <div className="min-w-0">
                              <p className="line-clamp-2 break-words text-[12px] font-semibold text-[var(--scry-ink2)]">
                                {releaseLabel}
                              </p>
                              <div className="mt-2 flex flex-wrap items-center gap-2 text-[11px] text-[var(--scry-muted3)]">
                                {attemptedAtLabel ? (
                                  <span>{attemptedAtLabel}</span>
                                ) : null}
                                {entry.sourceHint ? (
                                  <span>{entry.sourceHint}</span>
                                ) : null}
                              </div>
                            </div>
                            {entry.episodeIds.length > 0 ? (
                              <span className="shrink-0 rounded-[6px] border border-[var(--scry-line3)] bg-[var(--scry-card)] px-2 py-0.5 text-[10.5px] font-semibold text-[var(--scry-muted2)]">
                                {entry.episodeIds.length}
                              </span>
                            ) : null}
                          </div>
                          {entry.errorMessage ? (
                            <p className="mt-2 line-clamp-3 rounded-[8px] bg-red-500/10 px-2.5 py-1.5 text-[11px] leading-4 text-red-700 dark:text-red-200">
                              {entry.errorMessage}
                            </p>
                          ) : null}
                        </div>
                      );
                    })}
                  </div>
                </CollapsibleContent>
              </section>
            </Collapsible>
          )}
        </div>
      </div>
      <TitleHistoryModal
        open={historyOpen}
        onOpenChange={setHistoryOpen}
        titleId={title.id}
        titleName={title.name}
      />
    </aside>
  );
}

function isMediaSettingsSection(section: ContentSettingsSection): boolean {
  return (
    section === "library" ||
    section === "general" ||
    section === "quality" ||
    section === "renaming" ||
    section === "routing"
  );
}

function canAccessMediaSettingsSection(
  section: ContentSettingsSection,
  canManageConfig: boolean,
  canManageLibrarySettings: boolean,
): boolean {
  if (!isMediaSettingsSection(section)) {
    return true;
  }

  if (section === "library") {
    return canManageConfig || canManageLibrarySettings;
  }

  return canManageConfig;
}

export function MediaContentView({
  state,
}: {
  state: {
    view: ViewId;
    contentSettingsSection: ContentSettingsSection;
    canManageConfig: boolean;
    canManageSystemSettings: boolean;
    canManageCatalogSettings: boolean;
    canManageLibrarySettings: boolean;
    contentSettingsLabel: string;
    moviesPath: string;
    setMoviesPath: (value: string) => void;
    seriesPath: string;
    setSeriesPath: (value: string) => void;
    localPathStyle: LocalPathStyle | undefined;
    mediaSettingsLoading: boolean;
    librarySettingsSaving: boolean;
    qualityProfiles: ParsedQualityProfile[];
    qualityProfileEntries: ParsedQualityProfileEntry[];
    qualityProfileParseError: string;
    globalQualityProfileId: string;
    globalScoringPersona: ScoringPersonaId;
    categoryQualityProfileOverrides: Record<ViewCategoryId, string>;
    categoryRequiredAudioLanguages: Record<ViewCategoryId, string[]>;
    saveCategoryRequiredAudioLanguages: (
      languages: string[],
    ) => Promise<void> | void;
    categoryPersonaSelections: Record<
      ViewCategoryId,
      FacetScoringPersonaSelectionRecord
    >;
    activeQualityScopeId: ViewCategoryId;
    categoryFolderTemplates: Record<ViewCategoryId, string>;
    setCategoryFolderTemplates: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    categoryRenameTemplates: Record<ViewCategoryId, string>;
    setCategoryRenameTemplates: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    categoryRenameEnabled: Record<ViewCategoryId, string>;
    setCategoryRenameEnabled: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    categoryRenameCollisionPolicies: Record<ViewCategoryId, string>;
    setCategoryRenameCollisionPolicies: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    categoryRenameMissingMetadataPolicies: Record<ViewCategoryId, string>;
    setCategoryRenameMissingMetadataPolicies: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    categoryFillerPolicies: Record<ViewCategoryId, string>;
    setCategoryFillerPolicies: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    categoryRecapPolicies: Record<ViewCategoryId, string>;
    setCategoryRecapPolicies: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    categoryMonitorSpecials: Record<ViewCategoryId, string>;
    setCategoryMonitorSpecials: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    categoryInterSeasonMovies: Record<ViewCategoryId, string>;
    setCategoryInterSeasonMovies: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    categoryMonitorFillerMovies: Record<ViewCategoryId, string>;
    setCategoryMonitorFillerMovies: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    nfoWriteOnImport: Record<ViewCategoryId, string>;
    setNfoWriteOnImport: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    plexmatchWriteOnImport: Record<ViewCategoryId, string>;
    setPlexmatchWriteOnImport: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, string>>
    >;
    importMode: Record<ViewCategoryId, ImportMode>;
    setImportMode: React.Dispatch<
      React.SetStateAction<Record<ViewCategoryId, ImportMode>>
    >;
    qualityProfileInheritValue: string;
    toProfileOptions: (
      profiles: ParsedQualityProfile[],
    ) => QualityProfileOption[];
    handleFacetPersonaSave: (
      persona: ScoringPersonaId | null,
    ) => Promise<void> | void;
    saveSetting: (
      scope: string,
      scopeId: string | undefined,
      keyName: string,
      value: string,
    ) => void;
    saveCategoryQualityProfileOverride: (value: string) => Promise<void> | void;
    updateCategoryMediaProfileSettings: (
      event: React.FormEvent<HTMLFormElement>,
    ) => Promise<void> | void;
    mediaSettingsSaving: boolean;
    titleNameForQueue: string;
    setTitleNameForQueue: (value: string) => void;
    queueFacet: Facet;
    setQueueFacet: (value: Facet) => void;
    monitoredForQueue: boolean;
    setMonitoredForQueue: (value: boolean) => void;
    seasonFoldersForQueue: boolean;
    setSeasonFoldersForQueue: (value: boolean) => void;
    minAvailabilityForQueue: string;
    setMinAvailabilityForQueue: (value: string) => void;
    tvdbCandidates: TvdbSearchItem[];
    onAddSubmit: (
      event: React.FormEvent<HTMLFormElement>,
    ) => Promise<void> | void;
    addTvdbCandidateToCatalog: (
      candidate: TvdbSearchItem,
    ) => Promise<void> | void;
    titleFilter: string;
    setTitleFilter: (value: string) => void;
    refreshTitles: (query?: string) => Promise<void> | void;
    titleLoading: boolean;
    catalogTotalTitleCount: number;
    catalogHasMoreTitles: boolean;
    catalogLoadingMoreTitles: boolean;
    loadMoreCatalogTitles: () => Promise<void> | void;
    titleCatalogSortKey: TitleTableSortKey;
    titleCatalogSortDirection: TitleTableSortDirection;
    updateTitleCatalogSort: (key: TitleTableSortKey) => void;
    catalogBootstrapLoading: boolean;
    catalogInitialLoadComplete: boolean;
    monitoredTitles: TitleRecord[];
    titleContextTitles: TitleRecord[];
    titleQuickFilters: TitleQuickFilters;
    titleQuickFilterCounts: TitleQuickFilterCounts;
    toggleTitleQuickMonitoringFilter: (
      filter: "monitored" | "unmonitored",
    ) => void;
    toggleTitleQuickStatusFilter: (filter: "continuing" | "ended") => void;
    clearTitleQuickFilters: () => void;
    queueExisting: (title: TitleRecord) => Promise<void> | void;
    toggleTitleMonitored: (
      title: TitleRecord,
      monitored: boolean,
    ) => Promise<void> | void;
    runInteractiveSearchForTitle: (
      title: TitleRecord,
    ) => Promise<Release[]> | Release[];
    queueExistingFromRelease: (
      title: TitleRecord,
      release: Release,
    ) => Promise<void> | void;
    queueAdditionalFromRelease: (
      title: TitleRecord,
      release: Release,
    ) => Promise<void> | void;
    isTogglingTitleMonitoredById: Record<string, boolean>;
    downloadClients: DownloadClientRecord[];
    activeScopeRouting: ScopeRoutingRecord;
    activeScopeRoutingOrder: string[];
    downloadClientRoutingLoading: boolean;
    downloadClientRoutingSaving: boolean;
    updateDownloadClientRoutingForScope: (
      clientId: string,
      nextValue: Partial<NzbgetCategoryRoutingSettings>,
      options?: { save?: boolean },
    ) => Promise<void> | void;
    moveDownloadClientInScope: (
      clientId: string,
      direction: "up" | "down",
    ) => void;
    indexers: IndexerRecord[];
    activeScopeIndexerRouting: IndexerRoutingRecord;
    activeScopeIndexerRoutingOrder: string[];
    indexerRoutingLoading: boolean;
    indexerRoutingSaving: boolean;
    setIndexerEnabledForScope: (
      indexerId: string,
      enabled: boolean,
    ) => Promise<void> | void;
    updateIndexerRoutingForScope: (
      indexerId: string,
      nextValue: Partial<IndexerCategoryRoutingSettings>,
    ) => Promise<void> | void;
    moveIndexerInScope: (indexerId: string, direction: "up" | "down") => void;
    ruleSets: RuleSetRecord[];
    rulesLoading: boolean;
    rulesSaving: boolean;
    onToggleRuleFacet: (ruleSetId: string, enabled: boolean) => void;
    libraryScanLoading: boolean;
    libraryScanDisabled: boolean;
    libraryScanNotice: string | null;
    libraryScanSummary: LibraryScanSummary | null;
    libraries: LibraryRecord[];
    librariesLoading: boolean;
    rootValidationLibraries: LibraryRecord[];
    rootValidationLibrariesLoading: boolean;
    invalidRootLibraryIds: string[];
    selectedLibraryIds: string[];
    allLibrariesValue: string;
    setSelectedLibraryIds: (value: string[]) => void;
    libraryDownloadClients: DownloadClientRecord[];
    libraryDownloadClientsLoading: boolean;
    loadLibrarySettings: (
      libraryId: string,
    ) => Promise<LibrarySettingsRecord | null>;
    loadFacetDownloadClientRouting: (
      scopeId: Facet,
    ) => Promise<DownloadClientRoutingEntry[]>;
    createLibrary: (input: {
      name: string;
      roots: import("@/lib/types/titles").RootFolderOption[];
      settings?: LibrarySettingsDraft;
    }) => Promise<LibraryRecord | null | void> | LibraryRecord | null | void;
    updateLibrary: (
      libraryId: string,
      input: {
        name: string;
        roots: import("@/lib/types/titles").RootFolderOption[];
        settings?: LibrarySettingsDraft;
      },
    ) => Promise<LibraryRecord | null | void> | LibraryRecord | null | void;
    deleteLibrary: (
      libraryId: string,
    ) => Promise<boolean | void> | boolean | void;
    scanLibrary: (libraryId?: string) => Promise<void> | void;
    onOpenOverview: (
      targetView: ViewId,
      overviewTarget: OverviewTitleTarget,
    ) => void;
    selectedOverviewTitleId: string | null;
    selectedOverviewBlocklistEntries: TitleReleaseBlocklistEntry[];
    selectedOverviewExternalSubtitles: ExternalSubtitleRecord[];
    refreshSelectedOverviewExternalSubtitles: () => Promise<void> | void;
    deleteSelectedOverviewMediaFile: (
      title: TitleRecord,
      fileId: string,
    ) => void;
    makeSelectedOverviewMovieFilePrimary: (
      title: TitleRecord,
      fileId: string,
    ) => Promise<void> | void;
    selectedOverviewPrimaryMovieFileUpdatingId: string | null;
    previewTitleRename: (
      title: TitleRecord,
    ) => Promise<MediaRenamePlan | null> | MediaRenamePlan | null;
    applyTitleRename: (
      title: TitleRecord,
      plan: MediaRenamePlan,
    ) => Promise<boolean | void> | boolean | void;
    setSelectedOverviewTitleId: (titleId: string | null) => void;
    clearSelectedOverviewTitle: () => void;
    deleteCatalogTitle: (title: TitleRecord) => void;
    isDeletingCatalogTitleById: Record<string, boolean>;
    isMobile: boolean;
    viewMode: ContentViewMode;
    setViewMode: (value: ContentViewMode) => void;
    selectedTitleIds: ReadonlySet<string>;
    toggleTitleSelection: (titleId: string) => void;
    toggleAllVisibleTitles: (checked: boolean) => void;
    clearSelectedTitles: () => void;
    bulkActionBusy: boolean;
    bulkMonitorTitles: (monitored: boolean) => Promise<void> | void;
    openBulkTitleEdit: () => void;
    openBulkTitleDelete: () => void;
  };
}) {
  const t = useTranslate();
  const location = useLocation();
  const contextPanelViewportMatches = useMinViewportWidth(
    "(min-width: 760px)",
  );
  const posterContextPanelViewportMatches = useMinViewportWidth(
    "(min-width: 720px)",
  );
  const selectedTitleListInlineViewportMatches = useMinViewportWidth(
    "(min-width: 1180px)",
  );
  const selectedPosterInlineViewportMatches = useMinViewportWidth(
    `(min-width: ${SELECTED_POSTER_INLINE_MIN_WIDTH}px)`,
  );
  const {
    view,
    contentSettingsSection,
    canManageConfig,
    canManageSystemSettings,
    canManageCatalogSettings,
    canManageLibrarySettings,
    contentSettingsLabel,
    localPathStyle,
    mediaSettingsLoading,
    librarySettingsSaving,
    qualityProfiles,
    qualityProfileParseError,
    globalQualityProfileId,
    globalScoringPersona,
    categoryQualityProfileOverrides,
    categoryRequiredAudioLanguages,
    saveCategoryRequiredAudioLanguages,
    categoryPersonaSelections,
    activeQualityScopeId,
    categoryFolderTemplates,
    setCategoryFolderTemplates,
    categoryRenameTemplates,
    setCategoryRenameTemplates,
    categoryRenameEnabled,
    setCategoryRenameEnabled,
    categoryRenameCollisionPolicies,
    setCategoryRenameCollisionPolicies,
    categoryRenameMissingMetadataPolicies,
    setCategoryRenameMissingMetadataPolicies,
    categoryFillerPolicies,
    setCategoryFillerPolicies,
    categoryRecapPolicies,
    setCategoryRecapPolicies,
    categoryMonitorSpecials,
    setCategoryMonitorSpecials,
    categoryInterSeasonMovies,
    setCategoryInterSeasonMovies,
    categoryMonitorFillerMovies,
    setCategoryMonitorFillerMovies,
    nfoWriteOnImport,
    setNfoWriteOnImport,
    plexmatchWriteOnImport,
    setPlexmatchWriteOnImport,
    importMode,
    setImportMode,
    qualityProfileInheritValue,
    toProfileOptions,
    handleFacetPersonaSave,
    saveSetting,
    saveCategoryQualityProfileOverride,
    updateCategoryMediaProfileSettings,
    mediaSettingsSaving,
    titleNameForQueue,
    setTitleNameForQueue,
    queueFacet,
    setQueueFacet,
    monitoredForQueue,
    setMonitoredForQueue,
    seasonFoldersForQueue,
    setSeasonFoldersForQueue,
    minAvailabilityForQueue,
    setMinAvailabilityForQueue,
    tvdbCandidates,
    addTvdbCandidateToCatalog,
    onAddSubmit,
    titleFilter,
    setTitleFilter,
    refreshTitles,
    titleLoading,
    catalogTotalTitleCount,
    catalogHasMoreTitles,
    catalogLoadingMoreTitles,
    loadMoreCatalogTitles,
    titleCatalogSortKey,
    titleCatalogSortDirection,
    updateTitleCatalogSort,
    catalogBootstrapLoading,
    catalogInitialLoadComplete,
    monitoredTitles,
    titleContextTitles,
    titleQuickFilters,
    titleQuickFilterCounts,
    toggleTitleQuickMonitoringFilter,
    toggleTitleQuickStatusFilter,
    clearTitleQuickFilters,
    queueExisting,
    toggleTitleMonitored,
    runInteractiveSearchForTitle,
    queueExistingFromRelease,
    queueAdditionalFromRelease,
    isTogglingTitleMonitoredById,
    downloadClients,
    activeScopeRouting,
    activeScopeRoutingOrder,
    downloadClientRoutingLoading,
    downloadClientRoutingSaving,
    updateDownloadClientRoutingForScope,
    moveDownloadClientInScope,
    indexers,
    activeScopeIndexerRouting,
    activeScopeIndexerRoutingOrder,
    indexerRoutingLoading,
    indexerRoutingSaving,
    setIndexerEnabledForScope,
    updateIndexerRoutingForScope,
    moveIndexerInScope,
    libraryScanLoading,
    libraryScanDisabled,
    libraryScanNotice,
    libraryScanSummary,
    libraries,
    librariesLoading,
    libraryDownloadClients,
    libraryDownloadClientsLoading,
    rootValidationLibraries,
    rootValidationLibrariesLoading,
    invalidRootLibraryIds,
    selectedLibraryIds,
    allLibrariesValue,
    setSelectedLibraryIds,
    scanLibrary,
    onOpenOverview,
    selectedOverviewTitleId,
    selectedOverviewBlocklistEntries,
    selectedOverviewExternalSubtitles,
    refreshSelectedOverviewExternalSubtitles,
    deleteSelectedOverviewMediaFile,
    makeSelectedOverviewMovieFilePrimary,
    selectedOverviewPrimaryMovieFileUpdatingId,
    previewTitleRename,
    applyTitleRename,
    setSelectedOverviewTitleId,
    clearSelectedOverviewTitle,
    deleteCatalogTitle,
    isDeletingCatalogTitleById,
    viewMode,
    setViewMode,
    selectedTitleIds,
    toggleTitleSelection,
    toggleAllVisibleTitles,
    clearSelectedTitles,
    bulkActionBusy,
    bulkMonitorTitles,
    openBulkTitleEdit,
    openBulkTitleDelete,
  } = state;
  const [titleFilterInputValue, setTitleFilterInputValue] =
    React.useState(titleFilter);
  const [titleLayoutRef, titleLayoutWidth] =
    useMeasuredElementWidth<HTMLDivElement>();
  const deferredMonitoredTitles = React.useDeferredValue(monitoredTitles);
  const deferredTitleContextTitles =
    React.useDeferredValue(titleContextTitles);
  const deferredTitleRecommendationTitles = React.useMemo(
    () =>
      selectedLibraryIds.length === 0
        ? deferredTitleContextTitles
        : deferredTitleContextTitles.filter((title) =>
            selectedLibraryIds.includes(title.libraryId),
          ),
    [deferredTitleContextTitles, selectedLibraryIds],
  );
  const [visibleTitleTableColumns, setVisibleTitleTableColumns] =
    React.useState<TitleTableVisibleColumns>(() => ({
      ...DEFAULT_TITLE_TABLE_VISIBLE_COLUMNS,
    }));
  const titleTableColumnOptions = React.useMemo(
    () =>
      TITLE_TABLE_COLUMN_KEYS.filter(
        (key) => key !== "episodes" || view !== "movies",
      ),
    [view],
  );
  const toggleTitleTableColumn = React.useCallback(
    (key: TitleTableColumnKey, checked: boolean) => {
      setVisibleTitleTableColumns((current) => ({
        ...current,
        [key]: checked,
      }));
    },
    [],
  );

  React.useEffect(() => {
    setTitleFilterInputValue((current) =>
      current === titleFilter ? current : titleFilter,
    );
  }, [titleFilter]);
  const compactSelectedVisibleCount = React.useMemo(
    () =>
      deferredMonitoredTitles.filter((title) => selectedTitleIds.has(title.id))
        .length,
    [deferredMonitoredTitles, selectedTitleIds],
  );
  const selectedOverviewTitle = React.useMemo(
    () =>
      selectedOverviewTitleId
        ? (deferredTitleContextTitles.find(
            (title) => title.id === selectedOverviewTitleId,
          ) ?? null)
        : null,
    [deferredTitleContextTitles, selectedOverviewTitleId],
  );
  const activeOverviewTitle = selectedOverviewTitle;
  const activeOverviewTitleId = activeOverviewTitle?.id ?? null;
  React.useEffect(() => {
    if (!selectedOverviewTitleId || selectedOverviewTitle) {
      return;
    }
    if (
      titleLoading ||
      catalogBootstrapLoading ||
      !catalogInitialLoadComplete
    ) {
      return;
    }
    clearSelectedOverviewTitle();
  }, [
    catalogBootstrapLoading,
    catalogInitialLoadComplete,
    clearSelectedOverviewTitle,
    selectedOverviewTitle,
    selectedOverviewTitleId,
    titleLoading,
  ]);
  const handleSelectOverviewTitle = React.useCallback(
    (title: TitleRecord) => {
      setSelectedOverviewTitleId(title.id);
    },
    [setSelectedOverviewTitleId],
  );
  const effectiveContentSettingsSection = canAccessMediaSettingsSection(
    contentSettingsSection,
    canManageConfig,
    canManageLibrarySettings,
  )
    ? contentSettingsSection
    : canManageLibrarySettings &&
        !canManageConfig &&
        isMediaSettingsSection(contentSettingsSection)
      ? "library"
      : "overview";

  const scopeLabel =
    activeQualityScopeId === "movie"
      ? t("search.facetMovie")
      : activeQualityScopeId === "series"
        ? t("search.facetSeries")
        : t("search.facetAnime");
  const effectiveViewMode: ContentViewMode = viewMode;
  const contextPanelMinimumWidth =
    effectiveViewMode === "poster" ? 720 : 760;
  const contextPanelWidthMatches =
    titleLayoutWidth == null
      ? effectiveViewMode === "poster"
        ? posterContextPanelViewportMatches
        : contextPanelViewportMatches
      : titleLayoutWidth >= contextPanelMinimumWidth;
  const selectedOverviewTitleAvailable =
    selectedOverviewTitleId !== null && selectedOverviewTitle !== null;
  const contextPanelAvailable =
    contextPanelWidthMatches || selectedOverviewTitleAvailable;
  const selectedTitleLayoutActive =
    contextPanelAvailable && activeOverviewTitle !== null;
  const selectedTitlePosterInlineActive =
    selectedTitleLayoutActive &&
    effectiveViewMode === "poster" &&
    (titleLayoutWidth == null
      ? selectedPosterInlineViewportMatches
      : titleLayoutWidth >= SELECTED_POSTER_INLINE_MIN_WIDTH);
  const selectedTitleCompactLayoutActive =
    selectedTitleLayoutActive && !selectedTitlePosterInlineActive;
  const selectedTitleListInlineActive =
    selectedTitleCompactLayoutActive &&
    (titleLayoutWidth == null
      ? selectedTitleListInlineViewportMatches
      : titleLayoutWidth >= 1180);
  const [selectedTitleListDrawerOpen, setSelectedTitleListDrawerOpen] =
    React.useState(false);
  const selectedTitleListDrawerRef = React.useRef<HTMLDivElement | null>(null);
  const selectedTitleListPreviousFocusRef = React.useRef<HTMLElement | null>(
    null,
  );
  const selectedTitleListDrawerModeActive =
    selectedTitleListDrawerOpen &&
    selectedTitleCompactLayoutActive &&
    !selectedTitleListInlineActive;

  React.useEffect(() => {
    setSelectedTitleListDrawerOpen(false);
  }, [activeOverviewTitleId, selectedTitleCompactLayoutActive]);

  React.useEffect(() => {
    if (!selectedTitleListDrawerModeActive) {
      return;
    }

    selectedTitleListPreviousFocusRef.current =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;

    const getDrawerFocusableElements = () => {
      const drawer = selectedTitleListDrawerRef.current;
      if (!drawer) {
        return [];
      }

      return Array.from(
        drawer.querySelectorAll<HTMLElement>(
          'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
        ),
      ).filter((element) => element.offsetParent !== null);
    };

    const focusFrame = window.requestAnimationFrame(() => {
      const drawer = selectedTitleListDrawerRef.current;
      if (!drawer) {
        return;
      }

      (getDrawerFocusableElements()[0] ?? drawer).focus();
    });

    const handleSelectedTitleDrawerKeyDown = (event: KeyboardEvent) => {
      const drawer = selectedTitleListDrawerRef.current;
      const eventTarget = event.target;
      if (!(eventTarget instanceof Node) || !drawer?.contains(eventTarget)) {
        return;
      }

      if (event.key === "Escape") {
        event.preventDefault();
        setSelectedTitleListDrawerOpen(false);
        return;
      }

      if (event.key !== "Tab") {
        return;
      }

      const focusableElements = getDrawerFocusableElements();
      if (focusableElements.length === 0) {
        event.preventDefault();
        selectedTitleListDrawerRef.current?.focus();
        return;
      }

      const firstElement = focusableElements[0];
      const lastElement = focusableElements[focusableElements.length - 1];
      if (event.shiftKey && document.activeElement === firstElement) {
        event.preventDefault();
        lastElement.focus();
      } else if (!event.shiftKey && document.activeElement === lastElement) {
        event.preventDefault();
        firstElement.focus();
      }
    };

    window.addEventListener("keydown", handleSelectedTitleDrawerKeyDown);
    return () => {
      window.cancelAnimationFrame(focusFrame);
      window.removeEventListener("keydown", handleSelectedTitleDrawerKeyDown);
      const previousFocus = selectedTitleListPreviousFocusRef.current;
      selectedTitleListPreviousFocusRef.current = null;
      if (previousFocus && document.contains(previousFocus)) {
        previousFocus.focus();
      }
    };
  }, [selectedTitleListDrawerModeActive]);

  const isTitleCatalogView =
    view === "movies" || view === "series" || view === "anime";
  const selectedTitleCompactDrawerActive =
    selectedTitleCompactLayoutActive && !selectedTitleListInlineActive;
  const selectedTitleTableInlineActive =
    selectedTitleCompactLayoutActive && selectedTitleListInlineActive;
  const selectedTitleFullTableInlineActive =
    selectedTitleTableInlineActive && effectiveViewMode === "poster-table";
  const collectionViewMode: ContentViewMode = selectedTitleCompactDrawerActive
    ? "compact"
    : effectiveViewMode;
  const selectedTitlePosterLayoutActive =
    selectedTitleLayoutActive && collectionViewMode === "poster";
  const titleTablePaneWidth = resolveTitleTablePaneWidth({
    collectionViewMode,
    contextPanelAvailable,
    layoutWidth: titleLayoutWidth,
    selectedTitleLayoutActive,
    selectedTitleListInlineActive,
    selectedTitlePosterLayoutActive,
  });
  const effectiveVisibleTitleTableColumns = React.useMemo(
    () =>
      resolveEffectiveTitleTableColumns(
        visibleTitleTableColumns,
        titleTablePaneWidth,
        selectedTitleFullTableInlineActive,
      ),
    [
      selectedTitleFullTableInlineActive,
      titleTablePaneWidth,
      visibleTitleTableColumns,
    ],
  );
  const showTitleTableColumnControls =
    effectiveViewMode !== "poster" && !selectedTitleCompactDrawerActive;
  const showTitleBulkSelectionBar =
    !selectedTitleCompactLayoutActive &&
    compactSelectedVisibleCount > 0 &&
    (collectionViewMode === "compact" ||
      collectionViewMode === "poster-table") &&
    (titleTablePaneWidth == null || titleTablePaneWidth >= 780);

  React.useEffect(() => {
    if (
      !isTitleCatalogView ||
      collectionViewMode !== "poster" ||
      !catalogHasMoreTitles ||
      catalogLoadingMoreTitles
    ) {
      return;
    }

    const maybeLoadNextPage = () => {
      if (selectedTitlePosterLayoutActive) {
        const element = selectedTitleListDrawerRef.current;
        if (!element || element.clientHeight <= 0) {
          return;
        }
        const remaining =
          element.scrollHeight - (element.scrollTop + element.clientHeight);
        if (remaining <= 1200) {
          void loadMoreCatalogTitles();
        }
        return;
      }

      const scrollElement = document.documentElement;
      const remaining =
        scrollElement.scrollHeight - (window.scrollY + window.innerHeight);
      if (remaining <= 1200) {
        void loadMoreCatalogTitles();
      }
    };

    maybeLoadNextPage();
    const scrollElement = selectedTitlePosterLayoutActive
      ? selectedTitleListDrawerRef.current
      : window;
    scrollElement?.addEventListener("scroll", maybeLoadNextPage, {
      passive: true,
    });
    window.addEventListener("resize", maybeLoadNextPage);
    return () => {
      scrollElement?.removeEventListener("scroll", maybeLoadNextPage);
      window.removeEventListener("resize", maybeLoadNextPage);
    };
  }, [
    catalogHasMoreTitles,
    catalogLoadingMoreTitles,
    collectionViewMode,
    deferredMonitoredTitles.length,
    isTitleCatalogView,
    loadMoreCatalogTitles,
    selectedTitlePosterLayoutActive,
  ]);

  const selectedTitleListDrawerId =
    activeOverviewTitleId !== null
      ? `title-context-list-drawer-${activeOverviewTitleId}`
      : "title-context-list-drawer";
  const selectedTitleContextPanelId =
    activeOverviewTitleId !== null
      ? `title-context-panel-${activeOverviewTitleId}`
      : "title-context-panel";
  const contextPanelSelectedTitleId = contextPanelAvailable
    ? activeOverviewTitleId
    : null;
  const onSelectTitleForContextPanel = handleSelectOverviewTitle;
  const handleOpenOverviewFromContext = React.useCallback(
    (targetView: ViewId, overviewTarget: OverviewTitleTarget) => {
      if (effectiveViewMode === "poster") {
        persistOverviewWindowScroll(location.pathname);
      }
      onOpenOverview(targetView, overviewTarget);
    },
    [effectiveViewMode, location.pathname, onOpenOverview],
  );
  const explicitlySelectedLibraryIds = selectedLibraryIds.filter(
    (libraryId) => libraryId !== allLibrariesValue,
  );
  const selectedLibraryIdSet =
    explicitlySelectedLibraryIds.length > 0
      ? new Set(explicitlySelectedLibraryIds)
      : null;
  const relevantLibraries = selectedLibraryIdSet
    ? libraries.filter((library) => selectedLibraryIdSet.has(library.id))
    : libraries;
  const hasConfiguredRootFolders =
    !catalogInitialLoadComplete || librariesLoading
      ? null
      : relevantLibraries.some((library) =>
          library.roots.some((folder) => folder.path.trim().length > 0),
        );
  const hasInvalidConfiguredRootFolders =
    catalogInitialLoadComplete &&
    !librariesLoading &&
    relevantLibraries.some((library) =>
      invalidRootLibraryIds.includes(library.id),
    );
  const showInitialScanAction =
    canManageLibrarySettings &&
    catalogInitialLoadComplete &&
    monitoredTitles.length === 0 &&
    hasConfiguredRootFolders === true &&
    !hasInvalidConfiguredRootFolders;
  const showConfigureRootFoldersAction =
    canManageLibrarySettings &&
    catalogInitialLoadComplete &&
    monitoredTitles.length === 0 &&
    (hasConfiguredRootFolders === false || hasInvalidConfiguredRootFolders);
  const configureRootFoldersReason = hasInvalidConfiguredRootFolders
    ? "invalid"
    : "missing";
  const configureRootFoldersHref =
    view === "movies" || view === "series" || view === "anime"
      ? buildViewPath(view, undefined, "library")
      : undefined;

  const mediaLibrarySettingsTitle =
    view === "series"
      ? t("settings.seriesLibrarySettings")
      : view === "anime"
        ? t("settings.animeSettings")
        : t("settings.moviesLibrarySettings");

  const handleRenameTemplateChange = React.useCallback(
    (event: React.ChangeEvent<HTMLInputElement>) => {
      setCategoryRenameTemplates((previous) => ({
        ...previous,
        [activeQualityScopeId]: event.target.value,
      }));
    },
    [activeQualityScopeId, setCategoryRenameTemplates],
  );

  const handleFolderTemplateChange = React.useCallback(
    (event: React.ChangeEvent<HTMLInputElement>) => {
      setCategoryFolderTemplates((previous) => ({
        ...previous,
        [activeQualityScopeId]: event.target.value,
      }));
    },
    [activeQualityScopeId, setCategoryFolderTemplates],
  );

  const handleRenameCollisionPolicyChange = React.useCallback(
    (value: string) => {
      setCategoryRenameCollisionPolicies((previous) => ({
        ...previous,
        [activeQualityScopeId]: value,
      }));
    },
    [activeQualityScopeId, setCategoryRenameCollisionPolicies],
  );

  const handleRenameMissingMetadataPolicyChange = React.useCallback(
    (value: string) => {
      setCategoryRenameMissingMetadataPolicies((previous) => ({
        ...previous,
        [activeQualityScopeId]: value,
      }));
    },
    [activeQualityScopeId, setCategoryRenameMissingMetadataPolicies],
  );

  const handleFillerPolicyChange = React.useCallback(
    (value: string) => {
      setCategoryFillerPolicies((previous) => ({
        ...previous,
        [activeQualityScopeId]: value,
      }));
      saveSetting("system", activeQualityScopeId, "anime.filler_policy", value);
    },
    [activeQualityScopeId, setCategoryFillerPolicies, saveSetting],
  );

  const handleRecapPolicyChange = React.useCallback(
    (value: string) => {
      setCategoryRecapPolicies((previous) => ({
        ...previous,
        [activeQualityScopeId]: value,
      }));
      saveSetting("system", activeQualityScopeId, "anime.recap_policy", value);
    },
    [activeQualityScopeId, setCategoryRecapPolicies, saveSetting],
  );

  const handleMonitorSpecialsChange = React.useCallback(
    (checked: boolean) => {
      const value = checked ? "true" : "false";
      setCategoryMonitorSpecials((previous) => ({
        ...previous,
        [activeQualityScopeId]: value,
      }));
      saveSetting(
        "system",
        activeQualityScopeId,
        "anime.monitor_specials",
        value,
      );
    },
    [activeQualityScopeId, setCategoryMonitorSpecials, saveSetting],
  );

  const handleInterSeasonMoviesChange = React.useCallback(
    (checked: boolean) => {
      const value = checked ? "true" : "false";
      setCategoryInterSeasonMovies((previous) => ({
        ...previous,
        [activeQualityScopeId]: value,
      }));
      saveSetting(
        "system",
        activeQualityScopeId,
        "anime.inter_season_movies",
        value,
      );
    },
    [activeQualityScopeId, setCategoryInterSeasonMovies, saveSetting],
  );

  const handleMonitorFillerMoviesChange = React.useCallback(
    (checked: boolean) => {
      const value = checked ? "true" : "false";
      setCategoryMonitorFillerMovies((previous) => ({
        ...previous,
        [activeQualityScopeId]: value,
      }));
      saveSetting(
        "system",
        activeQualityScopeId,
        "anime.monitor_filler_movies",
        value,
      );
    },
    [activeQualityScopeId, setCategoryMonitorFillerMovies, saveSetting],
  );

  const handleNfoWriteChange = React.useCallback(
    (checked: boolean) => {
      const value = checked ? "true" : "false";
      const key =
        activeQualityScopeId === "movie"
          ? "nfo.write_on_import.movie"
          : activeQualityScopeId === "anime"
            ? "nfo.write_on_import.anime"
            : "nfo.write_on_import.series";
      setNfoWriteOnImport((previous) => ({
        ...previous,
        [activeQualityScopeId]: value,
      }));
      saveSetting("system", undefined, key, value);
    },
    [activeQualityScopeId, setNfoWriteOnImport, saveSetting],
  );

  const handlePlexmatchWriteChange = React.useCallback(
    (checked: boolean) => {
      const value = checked ? "true" : "false";
      const key =
        activeQualityScopeId === "anime"
          ? "plexmatch.write_on_import.anime"
          : "plexmatch.write_on_import.series";
      setPlexmatchWriteOnImport((previous) => ({
        ...previous,
        [activeQualityScopeId]: value,
      }));
      saveSetting("system", undefined, key, value);
    },
    [activeQualityScopeId, setPlexmatchWriteOnImport, saveSetting],
  );

  const handleImportModeChange = React.useCallback(
    (value: ImportMode) => {
      setImportMode((previous) => ({
        ...previous,
        [activeQualityScopeId]: value,
      }));
      saveSetting("system", activeQualityScopeId, "import.mode", value);
    },
    [activeQualityScopeId, saveSetting, setImportMode],
  );

  const handleIndexerCategoriesChange = React.useCallback(
    (indexerId: string, categories: string[]) => {
      void updateIndexerRoutingForScope(indexerId, {
        categories,
      });
    },
    [updateIndexerRoutingForScope],
  );

  const handleIndexerEnabledChange = React.useCallback(
    (indexerId: string, checked: boolean) => {
      void setIndexerEnabledForScope(indexerId, checked);
    },
    [setIndexerEnabledForScope],
  );

  const moveIndexerUp = React.useCallback(
    (indexerId: string) => {
      moveIndexerInScope(indexerId, "up");
    },
    [moveIndexerInScope],
  );

  const moveIndexerDown = React.useCallback(
    (indexerId: string) => {
      moveIndexerInScope(indexerId, "down");
    },
    [moveIndexerInScope],
  );

  const handleTitleFilterChange = React.useCallback(
    (event: React.ChangeEvent<HTMLInputElement>) => {
      const nextValue = event.target.value;
      setTitleFilterInputValue(nextValue);
      React.startTransition(() => {
        setTitleFilter(nextValue);
      });
    },
    [setTitleFilter],
  );

  const handleRefreshTitles = React.useCallback(() => {
    const nextQuery = titleFilterInputValue;
    if (titleFilter !== nextQuery) {
      React.startTransition(() => {
        setTitleFilter(nextQuery);
      });
    }
    void refreshTitles(nextQuery);
  }, [refreshTitles, setTitleFilter, titleFilter, titleFilterInputValue]);

  const handleSelectedOverviewBackToList = React.useCallback(() => {
    clearSelectedOverviewTitle();
    void refreshTitles(titleFilterInputValue);
  }, [clearSelectedOverviewTitle, refreshTitles, titleFilterInputValue]);

  const handleLibraryScan = React.useCallback(
    (libraryId?: string) => {
      void scanLibrary(libraryId);
    },
    [scanLibrary],
  );

  const quickFilterView =
    view === "movies" ? "movies" : view === "series" ? "series" : "anime";
  const hasActiveTitleDisplayFilters =
    titleFilter.trim().length > 0 ||
    hasActiveTitleQuickFilters(titleQuickFilters, quickFilterView);
  const showEmptyStateActions = !hasActiveTitleDisplayFilters;

  const handleDeleteCatalogTitle = React.useCallback(
    (title: TitleRecord) => {
      deleteCatalogTitle(title);
    },
    [deleteCatalogTitle],
  );
  const mediaTitle = mediaTitleLabel(view, t);
  const visibleTitleCount = deferredMonitoredTitles.length;
  const totalTitleCount = Math.max(
    titleQuickFilterCounts.all,
    catalogTotalTitleCount,
  );
  const titleSummaryNoun = (() => {
    if (view === "movies") {
      return totalTitleCount === 1 ? "movie" : "movies";
    }
    return view === "series" ? "series" : "anime";
  })();
  const totalManagedBytes = React.useMemo(
    () =>
      deferredMonitoredTitles.reduce(
        (total, title) => total + Math.max(0, title.sizeBytes ?? 0),
        0,
      ),
    [deferredMonitoredTitles],
  );
  const mediaSummary = [
    `${totalTitleCount.toLocaleString()} ${titleSummaryNoun}`,
    `${visibleTitleCount.toLocaleString()} shown${catalogHasMoreTitles ? "+" : ""}`,
    `${bytesToReadable(totalManagedBytes)} managed`,
  ].join(" · ");

  return (
    <div className="flex min-h-0 flex-col gap-4">
      {effectiveContentSettingsSection === "quality" ? (
        <QualitySettingsPanel
          contentSettingsLabel={contentSettingsLabel}
          mediaSettingsLoading={mediaSettingsLoading}
          mediaSettingsSaving={mediaSettingsSaving}
          qualityProfiles={qualityProfiles}
          qualityProfileParseError={qualityProfileParseError}
          categoryQualityProfileOverrides={categoryQualityProfileOverrides}
          categoryRequiredAudioLanguages={categoryRequiredAudioLanguages}
          saveCategoryRequiredAudioLanguages={
            saveCategoryRequiredAudioLanguages
          }
          activeQualityScopeId={activeQualityScopeId}
          globalScoringPersona={globalScoringPersona}
          categoryPersonaSelections={categoryPersonaSelections}
          qualityProfileInheritValue={qualityProfileInheritValue}
          toProfileOptions={toProfileOptions}
          saveCategoryQualityProfileOverride={
            saveCategoryQualityProfileOverride
          }
          onFacetPersonaSave={handleFacetPersonaSave}
        />
      ) : effectiveContentSettingsSection === "renaming" ? (
        <RenameSettingsPanel
          activeQualityScopeId={activeQualityScopeId}
          mediaSettingsLoading={mediaSettingsLoading}
          mediaSettingsSaving={mediaSettingsSaving}
          categoryFolderTemplates={categoryFolderTemplates}
          handleFolderTemplateChange={handleFolderTemplateChange}
          categoryRenameTemplates={categoryRenameTemplates}
          handleRenameTemplateChange={handleRenameTemplateChange}
          categoryRenameEnabled={categoryRenameEnabled}
          handleRenameEnabledChange={(checked) =>
            setCategoryRenameEnabled((previous) => ({
              ...previous,
              [activeQualityScopeId]: checked ? "true" : "false",
            }))
          }
          categoryRenameCollisionPolicies={categoryRenameCollisionPolicies}
          handleRenameCollisionPolicyChange={handleRenameCollisionPolicyChange}
          categoryRenameMissingMetadataPolicies={
            categoryRenameMissingMetadataPolicies
          }
          handleRenameMissingMetadataPolicyChange={
            handleRenameMissingMetadataPolicyChange
          }
          updateCategoryMediaProfileSettings={
            updateCategoryMediaProfileSettings
          }
        />
      ) : effectiveContentSettingsSection === "routing" ? (
        <div className="space-y-4">
          <IndexerRoutingPanel
            scopeLabel={scopeLabel}
            activeQualityScopeId={activeQualityScopeId}
            indexers={indexers}
            activeScopeIndexerRouting={activeScopeIndexerRouting}
            activeScopeIndexerRoutingOrder={activeScopeIndexerRoutingOrder}
            indexerRoutingLoading={indexerRoutingLoading}
            indexerRoutingSaving={indexerRoutingSaving}
            onEnabledChange={handleIndexerEnabledChange}
            onCategoriesChange={handleIndexerCategoriesChange}
            onMoveUp={moveIndexerUp}
            onMoveDown={moveIndexerDown}
          />
          <DownloadClientRoutingPanel
            scopeLabel={scopeLabel}
            downloadClients={downloadClients}
            activeScopeRouting={activeScopeRouting}
            activeScopeRoutingOrder={activeScopeRoutingOrder}
            downloadClientRoutingLoading={downloadClientRoutingLoading}
            downloadClientRoutingSaving={downloadClientRoutingSaving}
            updateDownloadClientRoutingForScope={
              updateDownloadClientRoutingForScope
            }
            moveDownloadClientInScope={moveDownloadClientInScope}
          />
        </div>
      ) : effectiveContentSettingsSection === "library" ? (
        view === "movies" || view === "series" || view === "anime" ? (
          <MediaLibrarySettingsPanel
            facet={
              view === "movies"
                ? "movie"
                : view === "series"
                  ? "series"
                  : "anime"
            }
            settingsTitle={mediaLibrarySettingsTitle}
            libraries={libraries}
            librariesLoading={librariesLoading}
            rootValidationLibraries={rootValidationLibraries}
            rootValidationLibrariesLoading={rootValidationLibrariesLoading}
            preferredLibraryId={
              selectedLibraryIds.length === 1
                ? selectedLibraryIds[0]
                : allLibrariesValue
            }
            allLibrariesValue={allLibrariesValue}
            loading={mediaSettingsLoading}
            saving={librarySettingsSaving}
            scanLoading={libraryScanLoading}
            scanNotice={libraryScanNotice}
            scanSummary={libraryScanSummary}
            localPathStyle={localPathStyle}
            qualityProfiles={qualityProfiles}
            downloadClients={libraryDownloadClients}
            downloadClientsLoading={libraryDownloadClientsLoading}
            canCreateLibrary={canManageCatalogSettings}
            canManageDownloadClientRouting={
              canManageSystemSettings || canManageCatalogSettings
            }
            loadLibrarySettings={state.loadLibrarySettings}
            loadFacetDownloadClientRouting={
              state.loadFacetDownloadClientRouting
            }
            onCreateLibrary={state.createLibrary}
            onUpdateLibrary={state.updateLibrary}
            onDeleteLibrary={state.deleteLibrary}
            onScan={handleLibraryScan}
          />
        ) : null
      ) : effectiveContentSettingsSection === "general" ? (
        <GeneralSettingsPanel
          activeQualityScopeId={activeQualityScopeId}
          mediaSettingsLoading={mediaSettingsLoading}
          categoryFillerPolicies={categoryFillerPolicies}
          handleFillerPolicyChange={handleFillerPolicyChange}
          categoryRecapPolicies={categoryRecapPolicies}
          handleRecapPolicyChange={handleRecapPolicyChange}
          categoryMonitorSpecials={categoryMonitorSpecials}
          handleMonitorSpecialsChange={handleMonitorSpecialsChange}
          categoryInterSeasonMovies={categoryInterSeasonMovies}
          handleInterSeasonMoviesChange={handleInterSeasonMoviesChange}
          categoryMonitorFillerMovies={categoryMonitorFillerMovies}
          handleMonitorFillerMoviesChange={handleMonitorFillerMoviesChange}
          nfoWriteOnImport={nfoWriteOnImport}
          handleNfoWriteChange={handleNfoWriteChange}
          plexmatchWriteOnImport={plexmatchWriteOnImport}
          handlePlexmatchWriteChange={handlePlexmatchWriteChange}
          importMode={importMode}
          handleImportModeChange={handleImportModeChange}
        />
      ) : view === "movies" || view === "series" || view === "anime" ? (
        <Card
          id={`media-overview-${view}`}
          className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-none border-0 bg-transparent p-0 shadow-none"
        >
          <CardContent className="flex min-h-0 flex-1 flex-col space-y-0 p-0">
            <div className="shrink-0 border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,var(--scry-surfD),transparent)] px-4 pb-0 pt-4 sm:px-5 lg:px-6">
              <div className="flex flex-col gap-4 xl:flex-row xl:items-start xl:justify-between">
                <div className="min-w-0">
                  <h1 className="text-[22px] font-bold leading-tight tracking-normal text-[var(--scry-ink2)]">
                    {mediaTitle}
                  </h1>
                  <p className="mt-1 text-[12.5px] text-[var(--scry-muted3)]">
                    {mediaSummary}
                  </p>
                </div>
                <div className="flex min-w-0 flex-1 flex-col gap-2.5 lg:flex-row lg:items-center xl:max-w-[800px] xl:justify-end">
                  <div className="relative min-w-[220px] flex-1 xl:max-w-[520px]">
                    <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-[var(--scry-muted2)]" />
                    <Input
                      placeholder={t("title.filterPlaceholder")}
                      value={titleFilterInputValue}
                      onChange={handleTitleFilterChange}
                      className="h-10 w-full rounded-[10px] border-[var(--scry-border2)] bg-[var(--scry-inset)] pl-9 text-[13px] text-[var(--scry-body)] shadow-none placeholder:text-[var(--scry-faint2)] focus-visible:ring-[var(--scry-focus)]"
                    />
                  </div>
                  <LibraryMultiSelect
                    libraries={libraries}
                    selectedLibraryIds={selectedLibraryIds}
                    onSelectedLibraryIdsChange={setSelectedLibraryIds}
                    disabled={librariesLoading || libraries.length === 0}
                    triggerClassName="h-10 w-full rounded-[10px] border-[var(--scry-border2)] bg-[var(--scry-inset)] text-[13px] text-[var(--scry-body)] shadow-none lg:w-[180px]"
                  />
                  <ToggleGroup
                    type="single"
                    value={effectiveViewMode}
                    onValueChange={(v) => {
                      if (
                        v === "compact" ||
                        v === "poster-table" ||
                        v === "poster"
                      ) {
                        setViewMode(v);
                      }
                    }}
                    size="sm"
                    aria-label={t("title.viewModeToggle")}
                    className="h-10 shrink-0 rounded-[10px] border border-[var(--scry-border2)] bg-[var(--scry-inset)] p-1 shadow-none"
                  >
                    <ToggleGroupItem
                      id={titleOverviewViewModeId(view, "compact")}
                      value="compact"
                      size="sm"
                      aria-label={t("title.viewModeCompact")}
                      title={t("title.viewModeCompact")}
                      className="h-8 w-8 rounded-lg px-0 text-[var(--scry-muted2)] transition hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)] data-[state=on]:!border-transparent data-[state=on]:!bg-[var(--scry-accent-grad)] data-[state=on]:!text-primary-foreground"
                    >
                      <TableIcon className="h-4 w-4" />
                    </ToggleGroupItem>
                    <ToggleGroupItem
                      id={titleOverviewViewModeId(view, "poster-table")}
                      value="poster-table"
                      size="sm"
                      aria-label={t("title.viewModePosterTable")}
                      title={t("title.viewModePosterTable")}
                      className="h-8 w-8 rounded-lg px-0 text-[var(--scry-muted2)] transition hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)] data-[state=on]:!border-transparent data-[state=on]:!bg-[var(--scry-accent-grad)] data-[state=on]:!text-primary-foreground"
                    >
                      <LayoutList className="h-4 w-4" />
                    </ToggleGroupItem>
                    <ToggleGroupItem
                      id={titleOverviewViewModeId(view, "poster")}
                      value="poster"
                      size="sm"
                      aria-label={t("title.viewModePoster")}
                      title={t("title.viewModePoster")}
                      className="h-8 w-8 rounded-lg px-0 text-[var(--scry-muted2)] transition hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)] data-[state=on]:!border-transparent data-[state=on]:!bg-[var(--scry-accent-grad)] data-[state=on]:!text-primary-foreground"
                    >
                      <LayoutGrid className="h-4 w-4" />
                    </ToggleGroupItem>
                  </ToggleGroup>
                  {showTitleTableColumnControls ? (
                    <Popover>
                      <PopoverTrigger asChild>
                        <Button
                          type="button"
                          variant="outline"
                          className="h-10 w-full rounded-[10px] border-[var(--scry-border2)] bg-[var(--scry-inset)] px-3 text-[13px] shadow-none lg:w-auto"
                          aria-label={t("title.columns")}
                        >
                          <Columns3 className="h-4 w-4" />
                          <span>{t("title.columns")}</span>
                        </Button>
                      </PopoverTrigger>
                      <PopoverContent
                        align="end"
                        className="w-56 rounded-[12px] border-[var(--scry-border2)] bg-[var(--scry-surfD)] p-2"
                      >
                        <div className="space-y-1">
                          {titleTableColumnOptions.map((columnKey) => (
                            <label
                              key={columnKey}
                              className="flex min-h-9 cursor-pointer items-center gap-2 rounded-[8px] px-2.5 text-[12px] font-medium text-[var(--scry-body)] transition hover:bg-[var(--scry-hover)]"
                            >
                              <Checkbox
                                checked={visibleTitleTableColumns[columnKey]}
                                onCheckedChange={(checked) =>
                                  toggleTitleTableColumn(
                                    columnKey,
                                    checked === true,
                                  )
                                }
                                aria-label={titleTableColumnLabel(
                                  columnKey,
                                  t,
                                )}
                                className="size-4 rounded-[5px] [&_svg]:size-3"
                              />
                              <span className="min-w-0 truncate">
                                {titleTableColumnLabel(columnKey, t)}
                              </span>
                            </label>
                          ))}
                        </div>
                      </PopoverContent>
                    </Popover>
                  ) : null}
                </div>
              </div>
              <div className="mt-4">
                <TitleQuickFilterBar
                  view={view}
                  filters={titleQuickFilters}
                  counts={titleQuickFilterCounts}
                  onToggleMonitoring={toggleTitleQuickMonitoringFilter}
                  onToggleStatus={toggleTitleQuickStatusFilter}
                  onClear={clearTitleQuickFilters}
                  trailingContent={
                    showTitleBulkSelectionBar ? (
                      <div className="flex h-12 w-full items-center justify-end gap-2 rounded-[12px] border border-[var(--scry-border2)] bg-[var(--scry-inset)] px-3 py-2 sm:w-[20rem]">
                        <span className="mr-1 whitespace-nowrap text-sm text-[var(--scry-muted3)]">
                          {t("title.bulkSelectionCount", {
                            count: compactSelectedVisibleCount,
                          })}
                        </span>
                        <TitleTableActionButton
                          tone="enabled"
                          label={t("title.monitorAction")}
                          onClick={() => void bulkMonitorTitles(true)}
                          disabled={bulkActionBusy}
                          className="rounded-md"
                        >
                          <Eye className="h-4 w-4" />
                        </TitleTableActionButton>
                        <TitleTableActionButton
                          tone="disabled"
                          label={t("title.unmonitorAction")}
                          onClick={() => void bulkMonitorTitles(false)}
                          disabled={bulkActionBusy}
                          className="rounded-md"
                        >
                          <EyeOff className="h-4 w-4" />
                        </TitleTableActionButton>
                        <TitleTableActionButton
                          tone="edit"
                          label={t("label.edit")}
                          onClick={openBulkTitleEdit}
                          disabled={bulkActionBusy}
                          className="rounded-md"
                        >
                          <Pencil className="h-4 w-4" />
                        </TitleTableActionButton>
                        <TitleTableActionButton
                          tone="delete"
                          label={t("label.delete")}
                          onClick={openBulkTitleDelete}
                          disabled={bulkActionBusy}
                          className="rounded-md"
                        >
                          <Trash2 className="h-4 w-4" />
                        </TitleTableActionButton>
                        <TitleTableActionButton
                          tone="neutral"
                          label={t("label.clear")}
                          onClick={clearSelectedTitles}
                          disabled={bulkActionBusy}
                          className="rounded-md"
                        >
                          <X className="h-4 w-4" />
                        </TitleTableActionButton>
                      </div>
                    ) : null
                  }
                />
              </div>
            </div>
            <div
              className={cn(
                "flex min-h-0 flex-1 flex-col bg-transparent",
                selectedTitleLayoutActive
                  ? "overflow-hidden p-2 sm:p-3 lg:p-4"
                  : "p-3 sm:p-4 lg:p-5",
              )}
            >
              {(() => {
                const isMovieView = view === "movies";
                const overviewTargetView = isMovieView
                  ? ("movies" as const)
                  : view === "anime"
                    ? ("anime" as const)
                    : ("series" as const);
                const resolvedProfileName = (() => {
                  const overrideId =
                    categoryQualityProfileOverrides[activeQualityScopeId];
                  const effectiveId =
                    !overrideId || overrideId === qualityProfileInheritValue
                      ? globalQualityProfileId
                      : overrideId;
                  return (
                    qualityProfiles.find((p) => p.id === effectiveId)?.name ??
                    formatQualityProfileFallback(effectiveId) ??
                    qualityProfiles[0]?.name ??
                    null
                  );
                })();
                let titleCollectionView: React.ReactNode;

                if (collectionViewMode === "poster") {
                  titleCollectionView = (
                    <PosterGrid
                      key={`${view}-poster-grid`}
                      titles={deferredMonitoredTitles}
                      catalogInitialLoadComplete={catalogInitialLoadComplete}
                      onOpenOverview={onOpenOverview}
                      selectedTitleId={contextPanelSelectedTitleId}
                      contextPanelId={selectedTitleContextPanelId}
                      onSelectTitle={onSelectTitleForContextPanel}
                      onDelete={handleDeleteCatalogTitle}
                      onAutoQueue={queueExisting}
                      isDeletingById={isDeletingCatalogTitleById}
                      overviewTargetView={overviewTargetView}
                      showScanLibraryAction={
                        showEmptyStateActions && showInitialScanAction
                      }
                      showConfigureRootsAction={
                        showEmptyStateActions && showConfigureRootFoldersAction
                      }
                      configureRootsReason={configureRootFoldersReason}
                      configureRootsHref={configureRootFoldersHref}
                      onScanLibrary={scanLibrary}
                      scanLibraryLoading={libraryScanLoading}
                      scanLibraryDisabled={libraryScanDisabled}
                      scanLibraryNotice={libraryScanNotice}
                    />
                  );
                } else if (collectionViewMode === "compact") {
                  titleCollectionView = (
                    <CompactTitleTable
                      key={`${view}-${selectedTitleCompactLayoutActive ? "selected" : "full"}-compact-title-table`}
                      view={view}
                      titles={deferredMonitoredTitles}
                      titleLoading={titleLoading || catalogBootstrapLoading}
                      catalogHasMoreTitles={catalogHasMoreTitles}
                      catalogLoadingMoreTitles={catalogLoadingMoreTitles}
                      catalogPagingEnabled={
                        !selectedTitleCompactLayoutActive ||
                        selectedTitleListInlineActive ||
                        selectedTitleListDrawerOpen
                      }
                      onCatalogEndReached={loadMoreCatalogTitles}
                      sortKey={titleCatalogSortKey}
                      sortDirection={titleCatalogSortDirection}
                      onSortChange={updateTitleCatalogSort}
                      visibleColumns={effectiveVisibleTitleTableColumns}
                      resolvedProfileName={resolvedProfileName}
                      qualityProfiles={qualityProfiles}
                      qualityProfilesLoading={mediaSettingsLoading}
                      onOpenOverview={onOpenOverview}
                      selectedTitleId={contextPanelSelectedTitleId}
                      contextPanelId={selectedTitleContextPanelId}
                      onSelectTitle={onSelectTitleForContextPanel}
                      onDelete={handleDeleteCatalogTitle}
                      onAutoQueue={queueExisting}
                      onToggleMonitored={toggleTitleMonitored}
                      onInteractiveSearch={runInteractiveSearchForTitle}
                      onQueueFromInteractive={queueExistingFromRelease}
                      onQueueAdditionalFromInteractive={
                        queueAdditionalFromRelease
                      }
                      isDeletingById={isDeletingCatalogTitleById}
                      isTogglingMonitoredById={isTogglingTitleMonitoredById}
                      selectedTitleIds={selectedTitleIds}
                      onToggleSelected={toggleTitleSelection}
                      onToggleSelectAll={toggleAllVisibleTitles}
                      bulkActionBusy={bulkActionBusy}
                      showScanLibraryAction={
                        showEmptyStateActions && showInitialScanAction
                      }
                      showConfigureRootsAction={
                        showEmptyStateActions && showConfigureRootFoldersAction
                      }
                      configureRootsReason={configureRootFoldersReason}
                      configureRootsHref={configureRootFoldersHref}
                      onScanLibrary={scanLibrary}
                      scanLibraryLoading={libraryScanLoading}
                      scanLibraryDisabled={libraryScanDisabled}
                      scanLibraryNotice={libraryScanNotice}
                    />
                  );
                } else {
                  titleCollectionView = (
                    <TitleTable
                      key={`${view}-${
                        selectedTitleFullTableInlineActive
                          ? "selected"
                          : "full"
                      }-poster-title-table`}
                      view={view}
                      titles={deferredMonitoredTitles}
                      titleLoading={titleLoading || catalogBootstrapLoading}
                      catalogHasMoreTitles={catalogHasMoreTitles}
                      catalogLoadingMoreTitles={catalogLoadingMoreTitles}
                      onCatalogEndReached={loadMoreCatalogTitles}
                      sortKey={titleCatalogSortKey}
                      sortDirection={titleCatalogSortDirection}
                      onSortChange={updateTitleCatalogSort}
                      visibleColumns={effectiveVisibleTitleTableColumns}
                      resolvedProfileName={resolvedProfileName}
                      qualityProfiles={qualityProfiles}
                      qualityProfilesLoading={mediaSettingsLoading}
                      onOpenOverview={onOpenOverview}
                      selectedTitleId={contextPanelSelectedTitleId}
                      selectedPaneMode={selectedTitleFullTableInlineActive}
                      contextPanelId={selectedTitleContextPanelId}
                      onSelectTitle={onSelectTitleForContextPanel}
                      onDelete={handleDeleteCatalogTitle}
                      onAutoQueue={queueExisting}
                      onToggleMonitored={toggleTitleMonitored}
                      onInteractiveSearch={runInteractiveSearchForTitle}
                      onQueueFromInteractive={queueExistingFromRelease}
                      onQueueAdditionalFromInteractive={
                        queueAdditionalFromRelease
                      }
                      isDeletingById={isDeletingCatalogTitleById}
                      isTogglingMonitoredById={isTogglingTitleMonitoredById}
                      selectedTitleIds={selectedTitleIds}
                      onToggleSelected={toggleTitleSelection}
                      onToggleSelectAll={toggleAllVisibleTitles}
                      bulkActionBusy={bulkActionBusy}
                      showScanLibraryAction={
                        showEmptyStateActions && showInitialScanAction
                      }
                      showConfigureRootsAction={
                        showEmptyStateActions && showConfigureRootFoldersAction
                      }
                      configureRootsReason={configureRootFoldersReason}
                      configureRootsHref={configureRootFoldersHref}
                      onScanLibrary={scanLibrary}
                      scanLibraryLoading={libraryScanLoading}
                      scanLibraryDisabled={libraryScanDisabled}
                      scanLibraryNotice={libraryScanNotice}
                    />
                  );
                }
                const contextPanelGridTemplateColumns =
                  contextPanelAvailable && !selectedTitleLayoutActive
                    ? collectionViewMode === "poster"
                      ? "minmax(0,1fr) clamp(320px,30%,440px)"
                      : "minmax(0,1fr) clamp(700px,50%,1030px)"
                    : undefined;
                const selectedTitleGridTemplateColumns =
                  selectedTitleListInlineActive || selectedTitlePosterLayoutActive
                    ? "minmax(0,1fr) clamp(700px,50%,1030px)"
                    : undefined;
                const titleListDisclosure =
                  selectedTitleCompactLayoutActive &&
                  !selectedTitleListInlineActive ? (
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      className="h-8 gap-2 rounded-[8px] border-[var(--scry-border2)] bg-[var(--scry-soft)] px-3 text-[12px] shadow-none"
                      aria-expanded={selectedTitleListDrawerOpen}
                      aria-controls={selectedTitleListDrawerId}
                      aria-label={
                        selectedTitleListDrawerOpen
                          ? t("title.hideTitleList")
                          : t("title.showTitleList")
                      }
                      title={
                        selectedTitleListDrawerOpen
                          ? t("title.hideTitleList")
                          : t("title.showTitleList")
                      }
                      onClick={() =>
                        setSelectedTitleListDrawerOpen((open) => !open)
                      }
                    >
                      <PanelLeftOpen className="h-4 w-4" />
                      <span>{t("title.contextListDisclosure")}</span>
                    </Button>
                  ) : undefined;
                const titleOverviewPaneClassName =
                  selectedTitleLayoutActive
                    ? selectedTitleListInlineActive ||
                      selectedTitlePosterLayoutActive
                      ? "flex h-full min-h-0"
                      : "flex min-h-0 flex-1"
                    : contextPanelAvailable
                      ? collectionViewMode === "poster"
                        ? "sticky top-4 flex max-h-[calc(100vh-11rem)]"
                        : "flex h-full"
                      : "hidden";
                const titleOverviewPane =
                  activeOverviewTitle && view !== "movies" ? (
                    <section
                      id={selectedTitleContextPanelId}
                      aria-label={t("title.contextPanelTitle")}
                      className={cn(
                        "min-h-0 w-full min-w-0 flex-col overflow-hidden rounded-[16px] border border-[var(--scry-border2)] bg-[var(--scry-surfD)] shadow-[0_18px_44px_rgba(15,23,42,0.10)]",
                        titleOverviewPaneClassName,
                      )}
                    >
                      <div
                        data-slot="title-context-scroll"
                        className="relative min-h-0 flex-1 overflow-y-auto p-4 sm:p-5"
                      >
                        {titleListDisclosure ? (
                          <div className="mb-3 flex items-center">
                            {titleListDisclosure}
                          </div>
                        ) : null}
                        <SeriesOverviewContainer
                          titleId={activeOverviewTitle.id}
                          onTitleNotFound={handleSelectedOverviewBackToList}
                          onBackToList={handleSelectedOverviewBackToList}
                          onTitleResolved={(resolvedTitle) => {
                            if (resolvedTitle.id !== activeOverviewTitle.id) {
                              setSelectedOverviewTitleId(resolvedTitle.id);
                            }
                          }}
                        />
                      </div>
                    </section>
                  ) : (
                    <TitleContextPanel
                      id={selectedTitleContextPanelId}
                      title={activeOverviewTitle}
                      titles={
                        activeOverviewTitle
                          ? deferredTitleContextTitles
                          : deferredTitleRecommendationTitles
                      }
                      view={view}
                      overviewTargetView={overviewTargetView}
                      resolvedProfileName={resolvedProfileName}
                      blocklistEntries={selectedOverviewBlocklistEntries}
                      externalSubtitles={selectedOverviewExternalSubtitles}
                      qualityProfiles={qualityProfiles}
                      qualityProfilesLoading={mediaSettingsLoading}
                      isTogglingMonitored={
                        activeOverviewTitle
                          ? isTogglingTitleMonitoredById[
                              activeOverviewTitle.id
                            ] === true
                          : false
                      }
                      isDeleting={
                        activeOverviewTitle
                          ? isDeletingCatalogTitleById[
                              activeOverviewTitle.id
                            ] === true
                          : false
                      }
                      onOpenOverview={handleOpenOverviewFromContext}
                      onToggleMonitored={toggleTitleMonitored}
                      onAutoQueue={queueExisting}
                      onRefreshTitles={handleRefreshTitles}
                      onRefreshSubtitles={refreshSelectedOverviewExternalSubtitles}
                      onDeleteMediaFile={deleteSelectedOverviewMediaFile}
                      onMakePrimaryMediaFile={
                        makeSelectedOverviewMovieFilePrimary
                      }
                      primaryMediaFileUpdatingId={
                        selectedOverviewPrimaryMovieFileUpdatingId
                      }
                      onPreviewRename={previewTitleRename}
                      onApplyRename={applyTitleRename}
                      refreshLoading={titleLoading || catalogBootstrapLoading}
                      onInteractiveSearch={runInteractiveSearchForTitle}
                      onQueueFromInteractive={queueExistingFromRelease}
                      onQueueAdditionalFromInteractive={
                        queueAdditionalFromRelease
                      }
                      bulkActionBusy={bulkActionBusy}
                      onDelete={handleDeleteCatalogTitle}
                      onClearSelection={clearSelectedOverviewTitle}
                      onSelectTitle={handleSelectOverviewTitle}
                      titleListDisclosure={titleListDisclosure}
                      className={titleOverviewPaneClassName}
                    />
                  );

                return (
                  <div
                    ref={titleLayoutRef}
                    className={cn(
                      selectedTitleLayoutActive
                        ? cn(
                            "relative min-h-0 gap-4",
                            selectedTitleListInlineActive ||
                              selectedTitlePosterLayoutActive
                              ? "grid h-full items-stretch"
                              : "flex min-h-0 flex-1 flex-col overflow-hidden",
                          )
                        : "grid min-h-0 gap-4",
                      !selectedTitleLayoutActive &&
                        (collectionViewMode === "poster"
                          ? "items-start"
                          : "h-full"),
                    )}
                    style={
                      selectedTitleGridTemplateColumns ||
                      contextPanelGridTemplateColumns
                        ? {
                            gridTemplateColumns:
                              selectedTitleGridTemplateColumns ??
                              contextPanelGridTemplateColumns,
                          }
                        : undefined
                    }
                  >
                    {selectedTitleCompactLayoutActive &&
                    selectedTitleListDrawerOpen &&
                    !selectedTitleListInlineActive ? (
                      <button
                        type="button"
                        className="absolute inset-0 z-10 bg-black/45 backdrop-blur-[2px]"
                        aria-label={t("label.close")}
                        onClick={() => setSelectedTitleListDrawerOpen(false)}
                      />
                    ) : null}
                    <div
                      id={selectedTitleListDrawerId}
                      ref={selectedTitleListDrawerRef}
                      role={
                        selectedTitleListDrawerModeActive ? "dialog" : "region"
                      }
                      aria-modal={
                        selectedTitleListDrawerModeActive ? true : undefined
                      }
                      aria-label={t("title.contextTitleList")}
                      tabIndex={selectedTitleListDrawerModeActive ? -1 : undefined}
                      className={cn(
                        "min-w-0",
                        collectionViewMode === "poster"
                          ? selectedTitlePosterLayoutActive
                            ? "h-full min-h-0 overflow-y-auto pr-1"
                            : ""
                          : "min-h-0",
                        selectedTitleCompactLayoutActive &&
                          (selectedTitleListInlineActive
                            ? "block min-h-0"
                            : selectedTitleListDrawerOpen
                              ? "absolute bottom-3 left-3 top-3 z-30 flex w-[min(360px,82%)] min-w-0 flex-col overflow-hidden rounded-[14px] border border-[var(--scry-border2)] bg-[var(--scry-surfD)] p-2 shadow-[0_24px_70px_rgba(0,0,0,0.62)] motion-safe:animate-in motion-safe:fade-in-0 motion-safe:slide-in-from-left-3"
                              : "hidden"),
                      )}
                    >
                      {titleCollectionView}
                    </div>
                    {titleOverviewPane}
                  </div>
                );
              })()}
            </div>
          </CardContent>
        </Card>
      ) : (
        <AddTitleForm
          titleNameForQueue={titleNameForQueue}
          setTitleNameForQueue={setTitleNameForQueue}
          queueFacet={queueFacet}
          setQueueFacet={setQueueFacet}
          monitoredForQueue={monitoredForQueue}
          setMonitoredForQueue={setMonitoredForQueue}
          seasonFoldersForQueue={seasonFoldersForQueue}
          setSeasonFoldersForQueue={setSeasonFoldersForQueue}
          minAvailabilityForQueue={minAvailabilityForQueue}
          setMinAvailabilityForQueue={setMinAvailabilityForQueue}
          onAddSubmit={onAddSubmit}
          tvdbCandidates={tvdbCandidates}
          addTvdbCandidateToCatalog={addTvdbCandidateToCatalog}
          titleFilter={titleFilter}
          onTitleFilterChange={handleTitleFilterChange}
          onRefreshTitles={handleRefreshTitles}
          titleLoading={titleLoading}
          monitoredTitles={monitoredTitles}
          onOpenOverview={onOpenOverview}
          queueExisting={queueExisting}
        />
      )}
    </div>
  );
}
