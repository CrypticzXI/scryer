import * as React from "react";
import { useLocation } from "react-router-dom";
import {
  ArrowUpRight,
  CalendarDays,
  Eye,
  EyeOff,
  Folder,
  HardDrive,
  LayoutGrid,
  LayoutList,
  Library,
  Loader2,
  Pencil,
  RefreshCw,
  Search,
  Sparkles,
  Trash2,
  X,
  Zap,
} from "lucide-react";
import { useTranslate } from "@/lib/context/translate-context";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { LibraryMultiSelect } from "@/components/common/library-multi-select";
import { SearchResultBuckets } from "@/components/common/release-search-results";
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
  TitleRecord,
} from "@/lib/types";
import type { ImportMode } from "@/lib/types/settings";
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
  TitleEpisodeProgressBar,
  bytesToReadable,
  formatTitleDate,
  resolveDisplayedQualityLabel,
  type TitleTableSortDirection,
  type TitleTableSortKey,
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

type Facet = "movie" | "series" | "anime";

type ParsedQualityProfile = {
  id: string;
  name: string;
};

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

function CompactTableIcon() {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 16 16"
      className="h-4 w-4"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <rect x="2" y="2.5" width="12" height="11" rx="1.5" />
      <path d="M2 6.5h12M2 10h12M6 2.5v11" />
    </svg>
  );
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

function TitleContextMetric({
  icon: Icon,
  label,
  value,
  children,
}: {
  icon: React.ComponentType<{ className?: string }>;
  label: string;
  value?: React.ReactNode;
  children?: React.ReactNode;
}) {
  return (
    <div className="min-w-0 rounded-[12px] border border-[var(--scry-border2)] bg-[var(--scry-inset)] px-3 py-2.5">
      <div className="mb-1.5 flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-normal text-[var(--scry-muted2)]">
        <Icon className="h-3.5 w-3.5" />
        <span>{label}</span>
      </div>
      <div className="min-w-0 text-[13px] font-semibold text-[var(--scry-ink2)]">
        {children ?? value}
      </div>
    </div>
  );
}

type TitleContextRecommendationGroup = {
  id: string;
  label: string;
  titles: TitleRecord[];
};

function titleContextDateScore(value: string | null | undefined): number {
  if (!value) {
    return 0;
  }
  const parsed = Date.parse(value);
  return Number.isNaN(parsed) ? 0 : parsed;
}

function titleContextRecommendationScore(title: TitleRecord): number {
  const ownedEpisodes = title.episodesOwned ?? 0;
  const totalEpisodes = title.episodesTotal ?? title.episodesMonitored ?? 0;
  const progressScore =
    totalEpisodes > 0 ? Math.min(10, (ownedEpisodes / totalEpisodes) * 10) : 0;
  const sizeScore =
    title.sizeBytes && title.sizeBytes > 0
      ? Math.min(10, Math.log10(title.sizeBytes))
      : 0;
  const yearScore =
    typeof title.year === "number" && Number.isFinite(title.year)
      ? Math.min(5, Math.max(0, (title.year - 1990) / 10))
      : 0;

  return (
    (title.monitored ? 6 : 0) +
    progressScore +
    sizeScore +
    yearScore +
    titleContextDateScore(title.createdAt) / 1_000_000_000_000
  );
}

function titlePrimaryGenre(title: TitleRecord): string | null {
  const genre = title.genres?.find((candidate) => candidate.trim().length > 0);
  return genre?.trim() || null;
}

function buildTitleContextRecommendationGroups(
  titles: TitleRecord[],
  t: Translate,
): TitleContextRecommendationGroup[] {
  if (titles.length === 0) {
    return [];
  }

  const rankedTitles = [...titles].sort(
    (a, b) =>
      titleContextRecommendationScore(b) - titleContextRecommendationScore(a),
  );
  const groups: TitleContextRecommendationGroup[] = [
    {
      id: "top",
      label: t("title.contextForYouTop"),
      titles: rankedTitles.slice(0, 4),
    },
  ];

  const genreCounts = new Map<string, { label: string; count: number }>();
  for (const title of titles) {
    for (const genre of title.genres ?? []) {
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
    const genreTitles = rankedTitles
      .filter((title) =>
        title.genres?.some(
          (genre) =>
            genre.trim().toLocaleLowerCase() ===
            topGenre.label.toLocaleLowerCase(),
        ),
      )
      .slice(0, 4);

    if (genreTitles.length > 0) {
      groups.push({
        id: "genre",
        label: t("title.contextForYouGenre", { genre: topGenre.label }),
        titles: genreTitles,
      });
    }
  }

  const recentTitles = [...titles]
    .sort(
      (a, b) =>
        titleContextDateScore(b.createdAt) - titleContextDateScore(a.createdAt),
    )
    .slice(0, 4);
  if (
    recentTitles.some((title) => titleContextDateScore(title.createdAt) > 0)
  ) {
    groups.push({
      id: "recent",
      label: t("title.contextForYouRecent"),
      titles: recentTitles,
    });
  }

  return groups;
}

function TitleContextRecommendationButton({
  title,
  view,
  t,
  onSelectTitle,
}: {
  title: TitleRecord;
  view: ViewId;
  t: Translate;
  onSelectTitle: (title: TitleRecord) => void;
}) {
  const posterUrl = selectPosterVariantUrl(title.posterUrl, "w70");
  const yearLabel = formatTitleYear(title);
  const statusLabel = localizedTitleStatus(t, title.contentStatus);
  const libraryLabel = title.libraryName ?? title.libraryId;
  const genreLabel = titlePrimaryGenre(title);
  const subline = [yearLabel, statusLabel, libraryLabel]
    .filter(Boolean)
    .join(" / ");

  return (
    <button
      type="button"
      className="group flex min-w-0 gap-3 rounded-[12px] border border-[var(--scry-border2)] bg-[var(--scry-inset)] p-2.5 text-left transition hover:border-[var(--scry-baccent)] hover:bg-[var(--scry-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-focus)]"
      aria-label={t("title.selectTitle", { name: title.name })}
      onClick={() => onSelectTitle(title)}
    >
      <div className="h-[72px] w-12 shrink-0 overflow-hidden rounded-[8px] border border-[var(--scry-border2)] bg-[var(--scry-soft)]">
        <TitlePosterSlot
          src={posterUrl}
          sourceSrc={title.posterSourceUrl}
          metadataFetchedAt={title.metadataFetchedAt}
          createdAt={title.createdAt}
          alt={t("media.posterAlt", { name: title.name })}
          className="h-full w-full object-cover"
          placeholderClassName="flex h-full w-full items-center justify-center px-1 text-center text-[9px] text-[var(--scry-muted3)]"
          emptyLabel={t("label.noArt")}
          loading="lazy"
          decoding="async"
        />
      </div>
      <div className="min-w-0 flex-1 py-0.5">
        <div className="flex min-w-0 items-start gap-2">
          <div className="min-w-0 flex-1">
            <p className="truncate text-[13px] font-semibold text-[var(--scry-ink2)]">
              {title.name}
            </p>
            <p className="mt-1 truncate text-[11.5px] text-[var(--scry-muted3)]">
              {subline || mediaTitleLabel(view, t)}
            </p>
          </div>
          <ArrowUpRight className="mt-0.5 h-3.5 w-3.5 shrink-0 text-[var(--scry-muted2)] transition group-hover:text-[var(--scry-accent)]" />
        </div>
        <div className="mt-2 flex flex-wrap gap-1.5">
          <span
            className={cn(
              "inline-flex h-5 items-center rounded-md border px-1.5 text-[10.5px] font-semibold",
              title.monitored
                ? "border-emerald-500/25 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
                : "border-rose-500/25 bg-rose-500/10 text-rose-700 dark:text-rose-300",
            )}
          >
            {title.monitored ? t("title.monitored") : t("title.unmonitored")}
          </span>
          {genreLabel ? (
            <span className="inline-flex h-5 items-center rounded-md border border-[var(--scry-border2)] bg-[var(--scry-soft)] px-1.5 text-[10.5px] font-semibold text-[var(--scry-muted2)]">
              {genreLabel}
            </span>
          ) : null}
        </div>
      </div>
    </button>
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
  right: TitleRecord,
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

function buildTitleMoreLikeThisTitles(
  title: TitleRecord,
  titles: TitleRecord[],
): TitleRecord[] {
  const titleGenres = titleNormalizedGenreSet(title);
  return titles
    .filter(
      (candidate) =>
        candidate.id !== title.id && candidate.facet === title.facet,
    )
    .map((candidate) => {
      const sharedGenres = titleSharedGenreCount(titleGenres, candidate);
      const sameLibrary = candidate.libraryId === title.libraryId ? 1 : 0;
      const monitored = candidate.monitored ? 1 : 0;
      return {
        candidate,
        score:
          sharedGenres * 8 +
          sameLibrary * 3 +
          monitored +
          titleContextRecommendationScore(candidate),
      };
    })
    .sort((left, right) => {
      const scoreDelta = right.score - left.score;
      return scoreDelta !== 0
        ? scoreDelta
        : left.candidate.name.localeCompare(right.candidate.name);
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
  onClick,
}: {
  icon: React.ComponentType<{ className?: string }>;
  label: string;
  loading?: boolean;
  destructive?: boolean;
  active?: boolean;
  disabled?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      className={cn(
        "flex h-[58px] min-w-0 flex-col items-center justify-center gap-1.5 bg-[var(--scry-card)] px-2 text-[10px] font-bold uppercase tracking-normal text-[var(--scry-muted3)] transition hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-focus)] disabled:cursor-not-allowed disabled:opacity-55",
        active
          ? "text-emerald-700 dark:text-emerald-300"
          : destructive
            ? "text-destructive hover:text-destructive"
            : "",
      )}
      disabled={disabled || loading}
      onClick={onClick}
    >
      {loading ? (
        <Loader2 className="h-4 w-4 animate-spin" />
      ) : (
        <Icon className="h-4 w-4" />
      )}
      <span className="max-w-full truncate">{label}</span>
    </button>
  );
}

function TitleContextMoreLikeThisStrip({
  titles,
  view,
  onSelectTitle,
}: {
  titles: TitleRecord[];
  view: ViewId;
  onSelectTitle: (title: TitleRecord) => void;
}) {
  const t = useTranslate();

  if (titles.length === 0) {
    return null;
  }

  return (
    <section>
      <h3 className="text-[11px] font-semibold uppercase tracking-normal text-[var(--scry-muted2)]">
        {t("title.contextMoreLikeThis")}
      </h3>
      <p className="mt-1 text-[11.5px] text-[var(--scry-muted3)]">
        {t("title.contextMoreLikeThisScope")}
      </p>
      <div className="mt-2 grid grid-cols-2 gap-2 min-[1500px]:grid-cols-3">
        {titles.map((similarTitle) => {
          const posterUrl = selectPosterVariantUrl(
            similarTitle.posterUrl,
            "w70",
          );
          const yearLabel = formatTitleYear(similarTitle);
          const genreLabel = titlePrimaryGenre(similarTitle);
          const subline = [yearLabel, genreLabel].filter(Boolean).join(" / ");

          return (
            <button
              key={similarTitle.id}
              type="button"
              className="group min-w-0 overflow-hidden rounded-[10px] border border-[var(--scry-border2)] bg-[var(--scry-inset)] text-left transition hover:border-[var(--scry-baccent)] hover:bg-[var(--scry-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-focus)]"
              aria-label={t("title.selectTitle", { name: similarTitle.name })}
              onClick={() => onSelectTitle(similarTitle)}
            >
              <div className="flex gap-2 p-2">
                <div className="h-[60px] w-10 shrink-0 overflow-hidden rounded-[7px] border border-[var(--scry-border2)] bg-[var(--scry-soft)]">
                  <TitlePosterSlot
                    src={posterUrl}
                    sourceSrc={similarTitle.posterSourceUrl}
                    metadataFetchedAt={similarTitle.metadataFetchedAt}
                    createdAt={similarTitle.createdAt}
                    alt={t("media.posterAlt", { name: similarTitle.name })}
                    className="h-full w-full object-cover"
                    placeholderClassName="flex h-full w-full items-center justify-center px-1 text-center text-[8px] text-[var(--scry-muted3)]"
                    emptyLabel={t("label.noArt")}
                    loading="lazy"
                    decoding="async"
                  />
                </div>
                <div className="min-w-0 flex-1 py-0.5">
                  <p className="truncate text-[12px] font-semibold text-[var(--scry-ink2)]">
                    {similarTitle.name}
                  </p>
                  <p className="mt-1 truncate text-[11px] text-[var(--scry-muted3)]">
                    {subline || mediaTitleLabel(view, t)}
                  </p>
                  <ArrowUpRight className="mt-2 h-3.5 w-3.5 text-[var(--scry-muted2)] transition group-hover:text-[var(--scry-accent)]" />
                </div>
              </div>
            </button>
          );
        })}
      </div>
    </section>
  );
}

function TitleContextForYouPanel({
  titles,
  view,
  onSelectTitle,
}: {
  titles: TitleRecord[];
  view: ViewId;
  onSelectTitle: (title: TitleRecord) => void;
}) {
  const t = useTranslate();
  const recommendationGroups = React.useMemo(
    () => buildTitleContextRecommendationGroups(titles, t),
    [titles, t],
  );

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-y-auto p-4">
      <div className="rounded-[14px] border border-[var(--scry-border2)] bg-[linear-gradient(135deg,rgba(var(--scry-accent-rgb),0.16),var(--scry-inset)_58%,var(--scry-surfD))] p-4">
        <div className="flex items-center gap-3">
          <div className="flex size-10 shrink-0 items-center justify-center rounded-[12px] border border-[var(--scry-baccent)] bg-[rgba(var(--scry-accent-rgb),0.14)] text-[var(--scry-accent)]">
            <Sparkles className="h-5 w-5" />
          </div>
          <div className="min-w-0">
            <p className="text-[15px] font-bold text-[var(--scry-ink2)]">
              {t("title.contextForYouTitle")}
            </p>
            <p className="mt-0.5 text-[12px] text-[var(--scry-muted3)]">
              {t("title.contextForYouSubtitle")}
            </p>
          </div>
        </div>
      </div>

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
              <h3 className="mb-2 text-[11px] font-semibold uppercase tracking-normal text-[var(--scry-muted2)]">
                {group.label}
              </h3>
              <div className="grid gap-2 min-[1536px]:grid-cols-2">
                {group.titles.map((recommendation) => (
                  <TitleContextRecommendationButton
                    key={`${group.id}-${recommendation.id}`}
                    title={recommendation}
                    view={view}
                    t={t}
                    onSelectTitle={onSelectTitle}
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
}) {
  const t = useTranslate();
  const requestIdRef = React.useRef(0);
  const [results, setResults] = React.useState<Release[] | null>(null);
  const [loading, setLoading] = React.useState(false);
  const [searchFailed, setSearchFailed] = React.useState(false);

  React.useEffect(() => {
    requestIdRef.current += 1;
    setResults(null);
    setLoading(false);
    setSearchFailed(false);
  }, [title.id]);

  const runSearch = React.useCallback(() => {
    if (disabled) {
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
  }, [disabled, onInteractiveSearch, title]);

  return (
    <section className="rounded-[12px] border border-[var(--scry-border2)] bg-[var(--scry-inset)] p-3">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <h3 className="text-[11px] font-semibold uppercase tracking-normal text-[var(--scry-muted2)]">
            {t("label.interactiveSearch")}
          </h3>
          <p className="mt-1 line-clamp-2 text-[12px] leading-5 text-[var(--scry-muted3)]">
            {t("help.interactiveSearchTooltip")}
          </p>
        </div>
        <Button
          type="button"
          size="sm"
          variant={results === null ? "secondary" : "ghost"}
          className="h-8 shrink-0 px-2.5 text-[12px]"
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
      </div>

      <div className="mt-3">
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
            compact
          />
        )}
      </div>
    </section>
  );
}

function TitleContextPanel({
  title,
  titles,
  view,
  overviewTargetView,
  resolvedProfileName,
  qualityProfiles,
  qualityProfilesLoading,
  isTogglingMonitored,
  isDeleting,
  onOpenOverview,
  onToggleMonitored,
  onAutoQueue,
  onInteractiveSearch,
  onQueueFromInteractive,
  onQueueAdditionalFromInteractive,
  bulkActionBusy,
  onDelete,
  onClearSelection,
  onSelectTitle,
  className,
}: {
  title: TitleRecord | null;
  titles: TitleRecord[];
  view: ViewId;
  overviewTargetView: ViewId;
  resolvedProfileName: string | null;
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
  className?: string;
}) {
  const t = useTranslate();
  const [autoQueueLoadingTitleId, setAutoQueueLoadingTitleId] = React.useState<
    string | null
  >(null);
  const panelClassName = cn(
    "hidden min-h-0 w-full min-w-0 flex-col overflow-hidden rounded-[16px] border border-[var(--scry-border2)] bg-[var(--scry-surfD)] shadow-[0_18px_44px_rgba(15,23,42,0.10)] xl:flex",
    className,
  );
  const moreLikeThisTitles = React.useMemo(
    () => (title ? buildTitleMoreLikeThisTitles(title, titles) : []),
    [title, titles],
  );

  if (!title) {
    return (
      <aside
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
    title.backgroundUrl ?? title.backgroundSourceUrl ?? null;
  const yearLabel = formatTitleYear(title);
  const statusLabel = localizedTitleStatus(t, title.contentStatus);
  const addedAtLabel = formatTitleDate(title.createdAt) ?? t("label.unknown");
  const unknownLabel = t("label.unknown");
  const qualityLabel = qualityProfilesLoading
    ? t("label.loading")
    : resolveDisplayedQualityLabel(
        title,
        qualityProfiles,
        resolvedProfileName,
        unknownLabel,
      );
  const rootFolderLabel = title.rootFolderPath?.trim() || t("label.unknown");
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
  const heroMetadataPills = [
    statusLabel,
    qualityLabel === unknownLabel || qualityLabel === loadingLabel
      ? null
      : qualityLabel,
    runtimeLabel,
    studioOrNetworkLabel,
  ].filter((value): value is string => Boolean(value));
  const heroGenreLabels = (title.genres ?? [])
    .map((genre) => genre.trim())
    .filter(Boolean)
    .slice(0, 4);
  const autoQueueLoading = autoQueueLoadingTitleId === title.id;
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

  return (
    <aside aria-label={t("title.contextPanelTitle")} className={panelClassName}>
      <div className="relative min-h-0 flex-1 overflow-y-auto">
        <div className="relative h-28 overflow-hidden border-b border-[var(--scry-border2)] bg-[var(--scry-inset)]">
          {backgroundUrl ? (
            <img
              src={backgroundUrl}
              alt=""
              aria-hidden="true"
              className="h-full w-full object-cover opacity-45"
              loading="lazy"
            />
          ) : null}
          <div className="absolute inset-0 bg-[linear-gradient(180deg,rgba(255,255,255,0.08),var(--scry-surfD))]" />
          <button
            type="button"
            aria-label={t("label.clear")}
            className="absolute right-3 top-3 z-10 flex size-8 items-center justify-center rounded-[10px] border border-white/35 bg-white/75 text-[var(--scry-ink2)] shadow-sm transition hover:bg-white focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-focus)] dark:border-white/15 dark:bg-slate-950/65 dark:hover:bg-slate-950"
            onClick={onClearSelection}
          >
            <X className="h-4 w-4" />
          </button>
        </div>
        <div className="-mt-14 px-4 pb-4">
          <div className="relative flex gap-3">
            <div className="h-36 w-24 shrink-0 overflow-hidden rounded-[12px] border border-[var(--scry-border2)] bg-[var(--scry-inset)] shadow-[0_12px_30px_rgba(15,23,42,0.18)]">
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
            </div>
            <div className="min-w-0 flex-1 pt-14">
              <div className="mb-2 flex flex-wrap items-center gap-1.5">
                <span
                  className={cn(
                    "inline-flex h-6 items-center gap-1 rounded-full border px-2 text-[11px] font-semibold",
                    title.monitored
                      ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
                      : "border-rose-500/30 bg-rose-500/10 text-rose-700 dark:text-rose-300",
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
                {heroMetadataPills.map((pill, index) => (
                  <span
                    key={`${index}-${pill}`}
                    className="inline-flex h-6 max-w-[11rem] items-center truncate rounded-full border border-[var(--scry-baccent)] bg-[rgba(var(--scry-accent-rgb),0.12)] px-2 text-[11px] font-semibold text-[var(--scry-accent)]"
                  >
                    {pill}
                  </span>
                ))}
              </div>
              <h2 className="line-clamp-3 text-[18px] font-bold leading-tight text-[var(--scry-ink2)]">
                {title.name}
              </h2>
              <p className="mt-1 truncate text-[12px] text-[var(--scry-muted3)]">
                {subheading || mediaTitleLabel(view, t)}
              </p>
              {heroGenreLabels.length > 0 ? (
                <div className="mt-2 flex flex-wrap gap-1.5">
                  {heroGenreLabels.map((genre) => (
                    <span
                      key={genre}
                      className="inline-flex h-6 max-w-[8.5rem] items-center rounded-md border border-[var(--scry-border2)] bg-[var(--scry-soft)] px-2 text-[11px] font-semibold text-[var(--scry-muted2)]"
                    >
                      <span className="min-w-0 truncate">{genre}</span>
                    </span>
                  ))}
                </div>
              ) : null}
            </div>
          </div>

          <div className="mt-4 grid grid-cols-4 gap-px overflow-hidden rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-border)]">
            <TitleContextActionButton
              icon={ArrowUpRight}
              label={t("title.openOverview")}
              disabled={bulkActionBusy}
              onClick={() => onOpenOverview(overviewTargetView, title)}
            />
            <TitleContextActionButton
              icon={Zap}
              label={t("title.queueLatest")}
              loading={autoQueueLoading}
              disabled={bulkActionBusy}
              onClick={() => void handleAutoQueue()}
            />
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
              onClick={() => void onToggleMonitored?.(title, !title.monitored)}
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

          <div className="mt-4 space-y-4">
            <section>
              <h3 className="text-[11px] font-semibold uppercase tracking-normal text-[var(--scry-muted2)]">
                {t("title.contextOverview")}
              </h3>
              <p className="mt-2 line-clamp-5 text-[12.5px] leading-5 text-[var(--scry-body)]">
                {overviewText}
              </p>
            </section>

            <div className="grid grid-cols-2 gap-2">
              <TitleContextMetric
                icon={Library}
                label={t("title.contextLibrary")}
                value={<span className="block truncate">{libraryLabel}</span>}
              />
              <TitleContextMetric
                icon={HardDrive}
                label={t("title.contextSize")}
                value={bytesToReadable(title.sizeBytes)}
              />
              <TitleContextMetric
                icon={Folder}
                label={t("title.contextRootFolder")}
                value={
                  <span className="block truncate">{rootFolderLabel}</span>
                }
              />
              <TitleContextMetric
                icon={CalendarDays}
                label={t("title.contextAdded")}
                value={addedAtLabel}
              />
            </div>

            <TitleContextMetric
              icon={LayoutList}
              label={t("title.contextQuality")}
              value={qualityLabel}
            />

            {view !== "movies" ? (
              <TitleContextMetric
                icon={LayoutGrid}
                label={t("title.contextEpisodes")}
              >
                <TitleEpisodeProgressBar item={title} t={t} compact />
              </TitleContextMetric>
            ) : null}

            <TitleContextReleaseSearchPanel
              title={title}
              onInteractiveSearch={onInteractiveSearch}
              onQueueFromInteractive={onQueueFromInteractive}
              onQueueAdditionalFromInteractive={
                onQueueAdditionalFromInteractive
              }
              disabled={bulkActionBusy}
            />

            <TitleContextMoreLikeThisStrip
              titles={moreLikeThisTitles}
              view={view}
              onSelectTitle={onSelectTitle}
            />
          </div>
        </div>
      </div>
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
    catalogHasMoreTitles: boolean;
    catalogLoadingMoreTitles: boolean;
    loadMoreCatalogTitles: () => Promise<void> | void;
    titleCatalogSortKey: TitleTableSortKey;
    titleCatalogSortDirection: TitleTableSortDirection;
    updateTitleCatalogSort: (key: TitleTableSortKey) => void;
    catalogBootstrapLoading: boolean;
    catalogInitialLoadComplete: boolean;
    monitoredTitles: TitleRecord[];
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
    "(min-width: 1280px)",
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
    catalogHasMoreTitles,
    catalogLoadingMoreTitles,
    loadMoreCatalogTitles,
    titleCatalogSortKey,
    titleCatalogSortDirection,
    updateTitleCatalogSort,
    catalogBootstrapLoading,
    catalogInitialLoadComplete,
    monitoredTitles,
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
    setSelectedOverviewTitleId,
    clearSelectedOverviewTitle,
    deleteCatalogTitle,
    isDeletingCatalogTitleById,
    isMobile,
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
  const deferredMonitoredTitles = React.useDeferredValue(monitoredTitles);

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
        ? (deferredMonitoredTitles.find(
            (title) => title.id === selectedOverviewTitleId,
          ) ?? null)
        : null,
    [deferredMonitoredTitles, selectedOverviewTitleId],
  );
  const activeOverviewTitle = selectedOverviewTitle;
  const activeOverviewTitleId = activeOverviewTitle?.id ?? null;
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
  const effectiveViewMode: ContentViewMode = isMobile ? "poster" : viewMode;
  const contextPanelAvailable = contextPanelViewportMatches && !isMobile;
  const contextPanelSelectedTitleId = contextPanelAvailable
    ? activeOverviewTitleId
    : null;
  const onSelectTitleForContextPanel = contextPanelAvailable
    ? handleSelectOverviewTitle
    : undefined;
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
  const libraryCount = libraries.length;
  const totalManagedBytes = React.useMemo(
    () =>
      deferredMonitoredTitles.reduce(
        (total, title) => total + Math.max(0, title.sizeBytes ?? 0),
        0,
      ),
    [deferredMonitoredTitles],
  );
  const mediaSummary = `${visibleTitleCount.toLocaleString()} shown${catalogHasMoreTitles ? "+" : ""} / ${libraryCount.toLocaleString()} ${libraryCount === 1 ? "library" : "libraries"} / ${bytesToReadable(totalManagedBytes)} managed`;

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
                      className="h-10 w-full rounded-[10px] border-[var(--scry-border2)] bg-[var(--scry-inset)] pl-9 text-[13px] text-[var(--scry-body)] shadow-none placeholder:text-[var(--scry-faint2)] focus-visible:ring-primary/35"
                    />
                  </div>
                  <LibraryMultiSelect
                    libraries={libraries}
                    selectedLibraryIds={selectedLibraryIds}
                    onSelectedLibraryIdsChange={setSelectedLibraryIds}
                    disabled={librariesLoading || libraries.length === 0}
                    triggerClassName="h-10 w-full rounded-[10px] border-[var(--scry-border2)] bg-[var(--scry-inset)] text-[13px] text-[var(--scry-body)] shadow-none lg:w-[180px]"
                  />
                  {!isMobile ? (
                    <ToggleGroup
                      type="single"
                      value={viewMode}
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
                        className="h-8 w-8 rounded-lg px-0 text-[var(--scry-muted2)] transition hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)] data-[state=on]:!border-transparent data-[state=on]:!bg-[var(--scry-accent-grad)] data-[state=on]:!text-primary-foreground data-[state=on]:!shadow-[0_8px_18px_rgba(var(--scry-accent-rgb),0.24)]"
                      >
                        <CompactTableIcon />
                      </ToggleGroupItem>
                      <ToggleGroupItem
                        id={titleOverviewViewModeId(view, "poster-table")}
                        value="poster-table"
                        size="sm"
                        aria-label={t("title.viewModePosterTable")}
                        title={t("title.viewModePosterTable")}
                        className="h-8 w-8 rounded-lg px-0 text-[var(--scry-muted2)] transition hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)] data-[state=on]:!border-transparent data-[state=on]:!bg-[var(--scry-accent-grad)] data-[state=on]:!text-primary-foreground data-[state=on]:!shadow-[0_8px_18px_rgba(var(--scry-accent-rgb),0.24)]"
                      >
                        <LayoutList className="h-4 w-4" />
                      </ToggleGroupItem>
                      <ToggleGroupItem
                        id={titleOverviewViewModeId(view, "poster")}
                        value="poster"
                        size="sm"
                        aria-label={t("title.viewModePoster")}
                        title={t("title.viewModePoster")}
                        className="h-8 w-8 rounded-lg px-0 text-[var(--scry-muted2)] transition hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)] data-[state=on]:!border-transparent data-[state=on]:!bg-[var(--scry-accent-grad)] data-[state=on]:!text-primary-foreground data-[state=on]:!shadow-[0_8px_18px_rgba(var(--scry-accent-rgb),0.24)]"
                      >
                        <LayoutGrid className="h-4 w-4" />
                      </ToggleGroupItem>
                    </ToggleGroup>
                  ) : null}
                  <Button
                    id={`title-overview-refresh-${view === "movies" ? "movie" : view === "series" ? "series" : "anime"}`}
                    className="h-10 w-full rounded-[10px] px-3 text-[13px] shadow-[0_10px_24px_rgba(var(--scry-accent-rgb),0.22)] lg:w-auto"
                    variant="primary"
                    onClick={handleRefreshTitles}
                    disabled={titleLoading}
                  >
                    <RefreshCw
                      className={
                        titleLoading ? "h-4 w-4 animate-spin" : "h-4 w-4"
                      }
                    />
                    <span>{t("label.refresh")}</span>
                  </Button>
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
                    effectiveViewMode === "compact" ||
                    effectiveViewMode === "poster-table" ? (
                      compactSelectedVisibleCount > 0 ? (
                        <div className="flex h-12 w-full items-center justify-end gap-2 rounded-[12px] border border-[var(--scry-border2)] bg-[var(--scry-inset)] px-3 py-2 shadow-[inset_0_1px_0_rgba(255,255,255,0.04)] sm:w-[20rem]">
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
                      ) : (
                        <div
                          className="h-12 w-full sm:w-[20rem]"
                          aria-hidden="true"
                        />
                      )
                    ) : null
                  }
                />
              </div>
            </div>
            <div className="min-h-0 flex-1 bg-transparent p-3 sm:p-4 lg:p-5">
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

                if (effectiveViewMode === "poster") {
                  titleCollectionView = (
                    <PosterGrid
                      key={`${view}-poster-grid`}
                      titles={deferredMonitoredTitles}
                      catalogInitialLoadComplete={catalogInitialLoadComplete}
                      isMovieView={isMovieView}
                      resolvedProfileName={resolvedProfileName}
                      qualityProfiles={qualityProfiles}
                      qualityProfilesLoading={mediaSettingsLoading}
                      onOpenOverview={onOpenOverview}
                      selectedTitleId={contextPanelSelectedTitleId}
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
                } else if (effectiveViewMode === "compact") {
                  titleCollectionView = (
                    <CompactTitleTable
                      key={`${view}-compact-title-table`}
                      view={view}
                      titles={deferredMonitoredTitles}
                      titleLoading={titleLoading || catalogBootstrapLoading}
                      catalogHasMoreTitles={catalogHasMoreTitles}
                      catalogLoadingMoreTitles={catalogLoadingMoreTitles}
                      onCatalogEndReached={loadMoreCatalogTitles}
                      sortKey={titleCatalogSortKey}
                      sortDirection={titleCatalogSortDirection}
                      onSortChange={updateTitleCatalogSort}
                      resolvedProfileName={resolvedProfileName}
                      qualityProfiles={qualityProfiles}
                      qualityProfilesLoading={mediaSettingsLoading}
                      onOpenOverview={onOpenOverview}
                      selectedTitleId={contextPanelSelectedTitleId}
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
                      key={`${view}-poster-title-table`}
                      view={view}
                      titles={deferredMonitoredTitles}
                      titleLoading={titleLoading || catalogBootstrapLoading}
                      catalogHasMoreTitles={catalogHasMoreTitles}
                      catalogLoadingMoreTitles={catalogLoadingMoreTitles}
                      onCatalogEndReached={loadMoreCatalogTitles}
                      sortKey={titleCatalogSortKey}
                      sortDirection={titleCatalogSortDirection}
                      onSortChange={updateTitleCatalogSort}
                      resolvedProfileName={resolvedProfileName}
                      qualityProfiles={qualityProfiles}
                      qualityProfilesLoading={mediaSettingsLoading}
                      onOpenOverview={onOpenOverview}
                      selectedTitleId={contextPanelSelectedTitleId}
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
                const contextPanelGridTemplateColumns = contextPanelAvailable
                  ? activeOverviewTitle
                    ? "minmax(300px,0.72fr) minmax(560px,min(56vw,920px))"
                    : effectiveViewMode === "poster"
                      ? "minmax(0,1fr) minmax(320px,min(30vw,440px))"
                      : "minmax(0,1fr) minmax(520px,min(48vw,860px))"
                  : undefined;

                return (
                  <div
                    className={cn(
                      "grid min-h-0 gap-4",
                      effectiveViewMode === "poster" ? "items-start" : "h-full",
                    )}
                    style={
                      contextPanelGridTemplateColumns
                        ? {
                            gridTemplateColumns:
                              contextPanelGridTemplateColumns,
                          }
                        : undefined
                    }
                  >
                    <div
                      className={cn(
                        "min-w-0",
                        effectiveViewMode === "poster" ? "" : "min-h-0",
                      )}
                    >
                      {titleCollectionView}
                    </div>
                    <TitleContextPanel
                      title={activeOverviewTitle}
                      titles={deferredMonitoredTitles}
                      view={view}
                      overviewTargetView={overviewTargetView}
                      resolvedProfileName={resolvedProfileName}
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
                      onInteractiveSearch={runInteractiveSearchForTitle}
                      onQueueFromInteractive={queueExistingFromRelease}
                      onQueueAdditionalFromInteractive={
                        queueAdditionalFromRelease
                      }
                      bulkActionBusy={bulkActionBusy}
                      onDelete={handleDeleteCatalogTitle}
                      onClearSelection={clearSelectedOverviewTitle}
                      onSelectTitle={handleSelectOverviewTitle}
                      className={
                        effectiveViewMode === "poster"
                          ? "xl:sticky xl:top-4 xl:max-h-[calc(100vh-11rem)]"
                          : "xl:h-full"
                      }
                    />
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
