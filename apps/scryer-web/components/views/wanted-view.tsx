import { Fragment, useEffect, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { FilterChipButton } from "@/components/common/filter-chip-button";
import { LibraryMultiSelect } from "@/components/common/library-multi-select";
import { TitleAutocompletePicker } from "@/components/common/title-autocomplete-picker";
import type { OverviewTitleTarget, ViewId, WantedSection } from "@/components/root/types";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import type { Translate } from "@/components/root/types";
import { buildViewPath } from "@/lib/utils/routing";
import { formatUiDateTime } from "@/lib/utils/date-format";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  ChevronDown,
  ChevronRight,
  Clock,
  Download,
  Filter,
  Gauge,
  History,
  ListChecks,
  Pause,
  Play,
  RefreshCw,
  RotateCcw,
  Search,
  X,
} from "lucide-react";
import { CutoffUnmetView } from "@/components/views/cutoff-unmet-view";
import type { CutoffUnmetItem } from "@/components/views/cutoff-unmet-view";
import type {
  PendingReleaseItem,
  PendingReleaseStatus,
  LibraryRecord,
  Release,
  ReleaseDecisionItem,
  TitleRecord,
  WantedItem,
  WantedMediaType,
  WantedSearchPhase,
  WantedStatus,
} from "@/lib/types";
import type { UiDateTimeFormat } from "@/lib/types/settings";
import { useIsMobile } from "@/lib/hooks/use-mobile";

type CutoffUnmetViewState = {
  items: CutoffUnmetItem[];
  loading: boolean;
  facetFilter: string | undefined;
  setFacetFilter: (v: string | undefined) => void;
  libraries: LibraryRecord[];
  librariesLoading: boolean;
  selectedLibraryIds: string[];
  setSelectedLibraryIds: (value: string[]) => void;
  autoSearchingId: string | null;
  interactiveSearchingId: string | null;
  activeInteractiveItemId: string | null;
  searchResultsByItemId: Record<string, Release[]>;
  bulkSearching: boolean;
  bulkProgress: { current: number; total: number } | null;
  triggerAutoSearch: (item: CutoffUnmetItem) => Promise<void>;
  triggerInteractiveSearch: (item: CutoffUnmetItem) => Promise<void>;
  queueRelease: (item: CutoffUnmetItem, release: Release) => Promise<void>;
  triggerBulkSearch: () => void;
  cancelBulkSearch: () => void;
};

type WantedViewState = {
  items: WantedItem[];
  total: number;
  loading: boolean;
  statusFilters: WantedStatus[];
  setStatusFilters: (v: WantedStatus[]) => void;
  mediaTypeFilters: WantedMediaType[];
  setMediaTypeFilters: (v: WantedMediaType[]) => void;
  latestDecisionCodeFilters: string[];
  setLatestDecisionCodeFilters: (v: string[]) => void;
  selectedTitle: TitleRecord | null;
  setSelectedTitle: (title: TitleRecord | null) => void;
  libraries: LibraryRecord[];
  librariesLoading: boolean;
  selectedLibraryIds: string[];
  setSelectedLibraryIds: (value: string[]) => void;
  offset: number;
  setOffset: (v: number) => void;
  limit: number;
  refreshItems: () => Promise<void>;
  expandedItemId: string | null;
  decisions: ReleaseDecisionItem[];
  decisionsLoading: boolean;
  loadDecisions: (id: string) => Promise<void>;
  triggerSearch: (id: string) => Promise<void>;
  pauseItem: (id: string) => Promise<void>;
  resumeItem: (id: string) => Promise<void>;
  resetItem: (id: string) => Promise<void>;
  triggerMismatchRecovery: (titleId: string) => Promise<void>;
};

const STATUS_OPTIONS: WantedStatus[] = ["wanted", "grabbed", "completed", "paused"];
const MEDIA_TYPE_OPTIONS: WantedMediaType[] = ["movie", "episode", "series_movie"];
const LATEST_DECISION_OPTIONS = [
  "title_mismatch",
  "quality_blocked",
  "upgrade_rejected",
  "pending_delay",
  "already_active",
 ] as const;

type WantedFilterOption<T extends string> = {
  value: T;
  label: string;
};

function normalizeMultiSelectValues<T extends string>(
  selectedValues: T[],
  allValues: readonly T[],
): T[] {
  const selectedSet = new Set(selectedValues);
  const normalized = allValues.filter((value) => selectedSet.has(value));
  return normalized.length === 0 || normalized.length === allValues.length ? [] : normalized;
}

function toggleMultiSelectValue<T extends string>(
  selectedValues: T[],
  value: T,
  allValues: readonly T[],
): T[] {
  const selectedSet = new Set(selectedValues.length > 0 ? selectedValues : allValues);
  if (selectedSet.has(value)) {
    selectedSet.delete(value);
  } else {
    selectedSet.add(value);
  }

  return normalizeMultiSelectValues(Array.from(selectedSet), allValues);
}

function WantedFilterSection<T extends string>({
  title,
  allLabel,
  options,
  selectedValues,
  onSelectedValuesChange,
}: {
  title: string;
  allLabel: string;
  options: WantedFilterOption<T>[];
  selectedValues: T[];
  onSelectedValuesChange: (values: T[]) => void;
}) {
  const implicitAllSelected = selectedValues.length === 0;

  return (
    <div className="flex flex-col gap-2">
      <p className="text-xs font-medium text-muted-foreground">{title}</p>
      <div className="flex flex-col gap-1">
        <button
          type="button"
          onClick={() => onSelectedValuesChange([])}
          className="flex items-center gap-2 rounded-md px-1.5 py-1 text-left text-sm hover:bg-accent/50"
        >
          <Checkbox checked={implicitAllSelected} className="pointer-events-none" />
          <span>{allLabel}</span>
        </button>
        {options.map((option) => {
          const checked = implicitAllSelected || selectedValues.includes(option.value);
          const implicitChecked = implicitAllSelected && !selectedValues.includes(option.value);
          return (
            <button
              key={option.value}
              type="button"
              onClick={() =>
                onSelectedValuesChange(
                  toggleMultiSelectValue(
                    selectedValues,
                    option.value,
                    options.map((entry) => entry.value),
                  ),
                )
              }
              className="flex items-center gap-2 rounded-md px-1.5 py-1 text-left text-sm hover:bg-accent/50"
            >
              <Checkbox checked={checked} className="pointer-events-none" />
              <span className={implicitChecked ? "text-muted-foreground" : undefined}>
                {option.label}
              </span>
            </button>
          );
        })}
      </div>
    </div>
  );
}

function WantedFiltersPopover({
  statusFilters,
  setStatusFilters,
  mediaTypeFilters,
  setMediaTypeFilters,
  latestDecisionCodeFilters,
  setLatestDecisionCodeFilters,
  onFilterChange,
}: {
  statusFilters: WantedStatus[];
  setStatusFilters: (values: WantedStatus[]) => void;
  mediaTypeFilters: WantedMediaType[];
  setMediaTypeFilters: (values: WantedMediaType[]) => void;
  latestDecisionCodeFilters: string[];
  setLatestDecisionCodeFilters: (values: string[]) => void;
  onFilterChange: () => void;
}) {
  const t = useTranslate();

  const activeFilterCount =
    statusFilters.length + mediaTypeFilters.length + latestDecisionCodeFilters.length;
  const statusOptions = STATUS_OPTIONS.map((value) => ({
    value,
    label: formatWantedStatus(value, t),
  }));
  const mediaTypeOptions = MEDIA_TYPE_OPTIONS.map((value) => ({
    value,
    label: formatWantedMediaType(value, t),
  }));
  const latestDecisionOptions = LATEST_DECISION_OPTIONS.map((value) => ({
    value,
    label: formatWantedDecisionCode(value, t),
  }));

  return (
    <Popover>
      <PopoverTrigger asChild>
        <FilterChipButton
          selected={activeFilterCount > 0}
          onClick={() => undefined}
          icon={<Filter className="h-3.5 w-3.5" />}
          className="w-full justify-between sm:w-auto"
        >
          <span>{t("label.filters")}</span>
          {activeFilterCount > 0 ? (
            <span className="rounded-full bg-background/20 px-1.5 py-0.5 text-[11px] leading-none">
              {activeFilterCount}
            </span>
          ) : null}
        </FilterChipButton>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-72 p-3">
        <div className="flex flex-col gap-4">
          <WantedFilterSection
            title={t("wanted.filterStatus")}
            allLabel={t("wanted.allStatuses")}
            options={statusOptions}
            selectedValues={statusFilters}
            onSelectedValuesChange={(values) => {
              setStatusFilters(values);
              onFilterChange();
            }}
          />
          <WantedFilterSection
            title={t("wanted.filterMediaType")}
            allLabel={t("wanted.allTypes")}
            options={mediaTypeOptions}
            selectedValues={mediaTypeFilters}
            onSelectedValuesChange={(values) => {
              setMediaTypeFilters(values);
              onFilterChange();
            }}
          />
          <WantedFilterSection
            title={t("wanted.filterLatestDecision")}
            allLabel={t("wanted.allDecisions")}
            options={latestDecisionOptions}
            selectedValues={latestDecisionCodeFilters}
            onSelectedValuesChange={(values) => {
              setLatestDecisionCodeFilters(values);
              onFilterChange();
            }}
          />
        </div>
      </PopoverContent>
    </Popover>
  );
}

type ReleaseDecisionExplanationEntry = {
  code: string;
  delta: number;
};

function formatWantedMediaType(mediaType: WantedMediaType, t: Translate) {
  const key: Record<WantedMediaType, string> = {
    movie: "wanted.type.movie",
    episode: "wanted.type.episode",
    series_movie: "wanted.type.seriesMovie",
  };
  return t(key[mediaType]);
}

function formatWantedStatus(status: WantedStatus, t: Translate) {
  const key: Record<WantedStatus, string> = {
    wanted: "wanted.status.wanted",
    grabbed: "wanted.status.grabbed",
    completed: "wanted.status.completed",
    paused: "wanted.status.paused",
  };
  return t(key[status]);
}

function formatWantedPhase(phase: WantedSearchPhase, t: Translate) {
  const key: Record<WantedSearchPhase, string> = {
    primary: "wanted.phase.primary",
    pre_release: "wanted.phase.preRelease",
    pre_air: "wanted.phase.preAir",
    secondary: "wanted.phase.secondary",
    long_tail: "wanted.phase.longTail",
  };
  return t(key[phase]);
}

function formatWantedDecisionCode(code: string, t: Translate) {
  const key = {
    eligible: "wanted.decision.eligible",
    title_mismatch: "wanted.decision.titleMismatch",
    quality_blocked: "wanted.decision.qualityBlocked",
    upgrade_rejected: "wanted.decision.upgradeRejected",
    pending_delay: "wanted.decision.pendingDelay",
    already_active: "wanted.decision.alreadyActive",
    accept_initial: "wanted.decision.acceptInitial",
    accept_upgrade: "wanted.decision.acceptUpgrade",
    reject_insufficient_delta: "wanted.decision.rejectInsufficientDelta",
    reject_cooldown: "wanted.decision.rejectCooldown",
    reject_not_allowed: "wanted.decision.rejectNotAllowed",
  }[code];
  return key ? t(key) : code;
}

function wantedItemContext(item: WantedItem, t: Translate) {
  if (item.mediaType === "series_movie") {
    return t("wanted.context.seriesMovie");
  }
  if (item.mediaType === "episode" && item.seasonNumber) {
    return t("wanted.context.seasonEpisode", {
      seasonNumber: item.seasonNumber,
    });
  }
  if (item.mediaType === "episode") {
    return t("wanted.context.episode");
  }
  return t("wanted.context.movie");
}

function wantedItemOverviewView(item: WantedItem): ViewId | null {
  switch (item.titleFacet) {
    case "movie":
      return "movies";
    case "series":
      return "series";
    case "anime":
      return "anime";
    default:
      return null;
  }
}

function wantedItemOverviewTarget(item: WantedItem): OverviewTitleTarget | null {
  const normalizedTitleId = item.titleId.trim();
  if (!normalizedTitleId) {
    return null;
  }

  const normalizedSlug = item.titleSlug?.trim() || null;
  return {
    id: normalizedTitleId,
    slug: normalizedSlug,
    libraryId: item.libraryId,
    librarySlug: item.librarySlug,
  };
}

function formatWantedEpisodeCode(item: WantedItem): string | null {
  if (item.mediaType !== "episode") {
    return null;
  }

  const seasonDigits = item.seasonNumber?.match(/\d+/)?.[0] ?? null;
  const episodeDigits = item.episodeNumber?.match(/\d+/)?.[0] ?? null;
  if (!seasonDigits || !episodeDigits) {
    return null;
  }

  return `S${seasonDigits.padStart(2, "0")}E${episodeDigits.padStart(2, "0")}`;
}

function wantedItemSubtitle(item: WantedItem, t: Translate): string {
  return formatWantedEpisodeCode(item) ?? wantedItemContext(item, t);
}

function statusBadge(status: WantedStatus, t: Translate) {
  const colors: Record<WantedStatus, string> = {
    wanted: "bg-blue-500/20 text-blue-400",
    grabbed: "bg-amber-500/20 text-amber-400",
    completed: "bg-green-500/20 text-green-400",
    paused: "bg-muted text-muted-foreground",
  };
  return (
    <span
      className={`inline-block rounded px-2 py-0.5 text-xs font-medium ${colors[status] ?? "bg-muted text-muted-foreground"}`}
    >
      {formatWantedStatus(status, t)}
    </span>
  );
}

function phaseBadge(phase: WantedSearchPhase, t: Translate) {
  const colors: Record<WantedSearchPhase, string> = {
    primary: "bg-green-500/20 text-green-400",
    pre_release: "bg-purple-500/20 text-purple-400",
    pre_air: "bg-purple-500/20 text-purple-400",
    secondary: "bg-yellow-500/20 text-yellow-400",
    long_tail: "bg-muted text-muted-foreground",
  };
  return (
    <span
      className={`inline-block rounded px-2 py-0.5 text-xs font-medium ${colors[phase] ?? "bg-muted text-muted-foreground"}`}
    >
      {formatWantedPhase(phase, t)}
    </span>
  );
}

function decisionBadge(code: string, t: Translate) {
  const colors: Record<string, string> = {
    eligible: "bg-green-500/20 text-green-400",
    title_mismatch: "bg-red-500/20 text-red-400",
    quality_blocked: "bg-red-500/20 text-red-400",
    upgrade_rejected: "bg-amber-500/20 text-amber-400",
    pending_delay: "bg-yellow-500/20 text-yellow-400",
    already_active: "bg-muted text-muted-foreground",
    download_client_unavailable: "bg-yellow-500/20 text-yellow-400",
    repack_group_mismatch: "bg-red-500/20 text-red-400",
    accept_initial: "bg-green-500/20 text-green-400",
    accept_upgrade: "bg-green-500/20 text-green-400",
    reject_insufficient_delta: "bg-red-500/20 text-red-400",
    reject_cooldown: "bg-amber-500/20 text-amber-400",
    reject_not_allowed: "bg-red-500/20 text-red-400",
  };
  return (
    <span
      className={`inline-block rounded px-2 py-0.5 text-xs font-medium ${colors[code] ?? "bg-muted text-muted-foreground"}`}
    >
      {formatWantedDecisionCode(code, t)}
    </span>
  );
}

function formatDate(iso: string | null, dateTimeFormat: UiDateTimeFormat) {
  return formatUiDateTime(iso, dateTimeFormat, { fallback: "—" });
}

function formatBytes(bytes: number | null) {
  if (bytes == null) return "—";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

function parseDecisionExplanation(
  explanationJson: string | null,
): ReleaseDecisionExplanationEntry[] {
  if (!explanationJson) return [];

  try {
    const parsed = JSON.parse(explanationJson);
    if (!Array.isArray(parsed)) return [];

    return parsed.flatMap((entry) => {
      if (
        !entry ||
        typeof entry !== "object" ||
        typeof entry.code !== "string" ||
        entry.code.trim().length === 0 ||
        typeof entry.delta !== "number" ||
        !Number.isFinite(entry.delta)
      ) {
        return [];
      }

      return [{ code: entry.code, delta: entry.delta }];
    });
  } catch {
    return [];
  }
}

function formatSignedDelta(delta: number) {
  return delta > 0 ? `+${delta}` : `${delta}`;
}

type PendingViewState = {
  items: PendingReleaseItem[];
  total: number;
  loading: boolean;
  hasMore: boolean;
  loadingMore: boolean;
  refreshItems: () => Promise<void>;
  loadMoreItems: () => Promise<void>;
  forceGrab: (id: string) => Promise<void>;
  dismiss: (id: string) => Promise<void>;
};

type WantedViewProps = {
  section: WantedSection;
  wantedState: WantedViewState;
  cutoffState: CutoffUnmetViewState;
  pendingState: PendingViewState;
  historyContent?: React.ReactNode;
  onOpenOverview?: (
    targetView: ViewId,
    overviewTarget: OverviewTitleTarget,
    episodeId?: string,
  ) => void;
};

export function WantedView({
  section,
  wantedState,
  cutoffState,
  pendingState,
  historyContent,
  onOpenOverview,
}: WantedViewProps) {
  const t = useTranslate();
  const wantedNav = [
    {
      section: "wanted" as const,
      label: t("wanted.title"),
      count: wantedState.total,
      icon: ListChecks,
    },
    {
      section: "cutoff" as const,
      label: t("cutoff.title"),
      count: cutoffState.items.length,
      icon: Gauge,
    },
    {
      section: "pending" as const,
      label: t("pending.title"),
      count: pendingState.items.length,
      icon: Clock,
    },
    {
      section: "history" as const,
      label: t("history.title"),
      count: null,
      icon: History,
    },
  ];

  return (
    <div className="md:flex md:h-full md:min-h-0 md:flex-row">
      <aside className="w-full shrink-0 border-b border-[var(--scry-border3)] bg-[var(--scry-surfF)] p-3 md:h-full md:w-[218px] md:overflow-y-auto md:border-b-0 md:border-r md:p-[22px_14px]">
        <div className="mb-3 flex items-center gap-2 px-2 text-[var(--scry-ink2)] md:mb-4">
          <ListChecks className="h-[18px] w-[18px] text-[var(--scry-accent-text)]" />
          <span className="text-[16px] font-bold">{t("nav.wanted")}</span>
        </div>
        <nav
          className="flex gap-2 overflow-x-auto pb-1 md:flex-col md:overflow-visible md:pb-0"
          aria-label={t("nav.wanted")}
        >
          {wantedNav.map((item) => {
            const Icon = item.icon;
            const active = section === item.section;
            return (
              <Link
                key={item.section}
                to={buildViewPath("wanted", undefined, undefined, undefined, item.section)}
                className={cn(
                  "flex h-9 shrink-0 items-center gap-2 rounded-[9px] px-3 text-[13px] font-medium text-[var(--scry-muted)] transition hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)] md:w-full",
                  active &&
                    "bg-[linear-gradient(90deg,rgba(var(--scry-accent-rgb),0.26),rgba(var(--scry-accent-rgb),0.08))] text-[var(--scry-ink2)] shadow-[inset_2px_0_0_var(--scry-accent-ring)]",
                )}
              >
                <Icon
                  className={cn(
                    "h-[17px] w-[17px] text-[var(--scry-muted2)]",
                    active && "text-[var(--scry-accent-text)]",
                  )}
                />
                <span className="whitespace-nowrap">{item.label}</span>
                {item.count === null ? null : (
                  <span
                    className={cn(
                      "ml-auto rounded-md bg-[var(--scry-chip)] px-1.5 py-0.5 text-[10.5px] font-semibold tabular-nums text-[var(--scry-muted3)]",
                      active && "bg-[rgba(var(--scry-accent-rgb),0.16)] text-[var(--scry-accent-text)]",
                    )}
                  >
                    {item.count}
                  </span>
                )}
              </Link>
            );
          })}
        </nav>
      </aside>
      <main className="space-y-4 md:flex md:h-full md:min-h-0 md:min-w-0 md:flex-1 md:flex-col md:gap-4 md:space-y-0">
        {section === "history" ? (
          historyContent ?? (
            <WantedItemsCard state={wantedState} onOpenOverview={onOpenOverview} />
          )
        ) : section === "cutoff" ? (
          <CutoffUnmetView state={cutoffState} />
        ) : section === "pending" ? (
          <PendingReleasesCard state={pendingState} />
        ) : (
          <WantedItemsCard state={wantedState} onOpenOverview={onOpenOverview} />
        )}
      </main>
    </div>
  );
}

function WantedItemsCard({
  state,
  onOpenOverview,
}: {
  state: WantedViewState;
  onOpenOverview?: (
    targetView: ViewId,
    overviewTarget: OverviewTitleTarget,
    episodeId?: string,
  ) => void;
}) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const isMobile = useIsMobile();
  const {
    items,
    total,
    loading,
    statusFilters,
    setStatusFilters,
    mediaTypeFilters,
    setMediaTypeFilters,
    latestDecisionCodeFilters,
    setLatestDecisionCodeFilters,
    selectedTitle,
    setSelectedTitle,
    libraries,
    librariesLoading,
    selectedLibraryIds,
    setSelectedLibraryIds,
    offset,
    setOffset,
    limit,
    refreshItems,
    expandedItemId,
    decisions,
    decisionsLoading,
    loadDecisions,
    triggerSearch,
    pauseItem,
    resumeItem,
    resetItem,
    triggerMismatchRecovery,
  } = state;
  const [expandedDecisionIds, setExpandedDecisionIds] = useState<Set<string>>(new Set());

  const hasPrev = offset > 0;
  const hasNext = offset + limit < total;
  const shouldScrollDesktopTable = !isMobile;

  useEffect(() => {
    setExpandedDecisionIds(new Set());
  }, [expandedItemId, decisions]);

  const toggleDecisionScoring = (decisionId: string) => {
    setExpandedDecisionIds((current) => {
      const next = new Set(current);
      if (next.has(decisionId)) {
        next.delete(decisionId);
      } else {
        next.add(decisionId);
      }
      return next;
    });
  };

  const openWantedItemOverview = (item: WantedItem) => {
    if (!onOpenOverview) {
      return;
    }

    const targetView = wantedItemOverviewView(item);
    const overviewTarget = wantedItemOverviewTarget(item);
    if (!targetView || !overviewTarget) {
      return;
    }

    onOpenOverview(targetView, overviewTarget, item.episodeId ?? undefined);
  };

  return (
    <Card
      className={
        shouldScrollDesktopTable
          ? "flex min-h-0 flex-1 flex-col overflow-hidden rounded-none border-0 bg-transparent shadow-none"
          : "overflow-hidden rounded-none border-0 bg-transparent shadow-none"
      }
    >
      <CardHeader className="border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,var(--scry-surfD),transparent)] px-4 py-4 sm:px-5">
        <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <CardTitle className="text-[22px] font-bold tracking-normal text-[var(--scry-ink2)]">
            {t("wanted.title")}
          </CardTitle>
          <Button
            className="h-10 w-full rounded-[10px] border-[var(--scry-border2)] bg-[var(--scry-inset)] px-3 text-[13px] text-[var(--scry-body)] shadow-none hover:bg-[var(--scry-hover)] sm:w-auto"
            size="sm"
            variant="secondary"
            onClick={() => void refreshItems()}
            disabled={loading}
          >
            <RefreshCw className="mr-1 h-3 w-3" />
            {loading ? t("wanted.refreshing") : t("label.refresh")}
          </Button>
        </div>
      </CardHeader>
      <CardContent
        className={
          shouldScrollDesktopTable
            ? "flex min-h-0 flex-1 flex-col space-y-3 bg-[color-mix(in_srgb,var(--scry-bg)_52%,transparent)] p-4 sm:p-5"
            : "space-y-4 bg-[color-mix(in_srgb,var(--scry-bg)_52%,transparent)] p-4 sm:p-5"
        }
      >
        <div className="flex flex-col gap-3 rounded-[14px] border border-[var(--scry-border3)] bg-[var(--scry-surfC)] p-3 sm:flex-row sm:flex-wrap sm:items-center">
          <TitleAutocompletePicker
            ariaLabel={t("wanted.filterTitle")}
            className="w-full sm:max-w-sm"
            placeholder={t("wanted.filterTitlePlaceholder")}
            selectedTitle={selectedTitle}
            selectedTitleId={selectedTitle?.id ?? null}
            onSelectedTitleChange={setSelectedTitle}
          />

          <LibraryMultiSelect
            libraries={libraries}
            selectedLibraryIds={selectedLibraryIds}
            onSelectedLibraryIdsChange={(libraryIds) => {
              setSelectedLibraryIds(libraryIds);
              setOffset(0);
            }}
            disabled={librariesLoading || libraries.length === 0}
            triggerClassName="h-10 w-full rounded-[10px] border-[var(--scry-border2)] bg-[var(--scry-inset)] text-[13px] text-[var(--scry-body)] shadow-none sm:w-[180px]"
          />

          <WantedFiltersPopover
            statusFilters={statusFilters}
            setStatusFilters={setStatusFilters}
            mediaTypeFilters={mediaTypeFilters}
            setMediaTypeFilters={setMediaTypeFilters}
            latestDecisionCodeFilters={latestDecisionCodeFilters}
            setLatestDecisionCodeFilters={setLatestDecisionCodeFilters}
            onFilterChange={() => setOffset(0)}
          />

          <span className="self-center text-sm font-medium text-[var(--scry-muted3)] sm:ml-auto">
            {t("wanted.totalCount", { count: total })}
          </span>
        </div>

        {isMobile ? (
          items.length === 0 && !loading ? (
            <p className="text-center text-[var(--scry-muted3)]">{t("wanted.noItems")}</p>
          ) : (
            <div className="space-y-3">
              {items.map((item) => (
                <div key={item.id} className="rounded-[14px] border border-[var(--scry-border2)] bg-[var(--scry-surfC)] p-3 shadow-[0_12px_28px_rgba(2,6,23,0.10)]">
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0 flex-1">
                      <button
                        type="button"
                        className="block text-left hover:text-foreground"
                        onClick={() => openWantedItemOverview(item)}
                      >
                        <p className="break-words text-sm font-medium text-foreground hover:underline">
                          {item.titleName ?? item.titleId.slice(0, 8)}
                        </p>
                        <p className="mt-1 text-xs text-muted-foreground">
                          {wantedItemSubtitle(item, t)}
                        </p>
                        <p className="mt-1 text-xs text-muted-foreground">
                          {item.libraryName ?? item.libraryId ?? "Library"}
                        </p>
                      </button>
                      <div className="mt-2 flex flex-wrap gap-2">
                        {statusBadge(item.status, t)}
                        {phaseBadge(item.searchPhase, t)}
                        <span className="rounded bg-muted px-2 py-0.5 text-xs text-muted-foreground">
                          {formatWantedMediaType(item.mediaType, t)}
                        </span>
                      </div>
                    </div>
                    <button
                      type="button"
                      className="p-0.5 text-muted-foreground hover:text-foreground"
                      onClick={() => void loadDecisions(item.id)}
                    >
                      {expandedItemId === item.id ? (
                        <ChevronDown className="h-4 w-4" />
                      ) : (
                        <ChevronRight className="h-4 w-4" />
                      )}
                    </button>
                  </div>
                  <div className="mt-3 grid grid-cols-2 gap-2 text-xs text-muted-foreground">
                    <div>
                      <span className="block">{t("wanted.colNextSearch")}</span>
                      <span className="text-foreground">
                        {formatDate(item.nextSearchAt, dateTimeFormat)}
                      </span>
                    </div>
                    <div>
                      <span className="block">{t("wanted.colLatestDecision")}</span>
                      <span className="text-foreground">
                        {item.latestReleaseDecision
                          ? formatWantedDecisionCode(
                              item.latestReleaseDecision.decisionCode,
                              t,
                            )
                          : "—"}
                      </span>
                    </div>
                    <div>
                      <span className="block">{t("wanted.colScore")}</span>
                      <span className="text-foreground">{item.currentScore ?? "—"}</span>
                    </div>
                    <div>
                      <span className="block">{t("wanted.colSearches")}</span>
                      <span className="text-foreground">{item.searchCount}</span>
                    </div>
                  </div>
                  <div className="mt-3 flex flex-wrap gap-2">
                    <Button size="sm" variant="secondary" className="flex-1" onClick={() => void triggerSearch(item.id)}>
                      <Search className="h-4 w-4" />
                      <span>{t("wanted.searchNow")}</span>
                    </Button>
                    {item.status === "paused" ? (
                      <Button size="sm" variant="secondary" className="flex-1" onClick={() => void resumeItem(item.id)}>
                        <Play className="h-4 w-4" />
                        <span>{t("wanted.resume")}</span>
                      </Button>
                    ) : (
                      <Button size="sm" variant="secondary" className="flex-1" onClick={() => void pauseItem(item.id)}>
                        <Pause className="h-4 w-4" />
                        <span>{t("wanted.pause")}</span>
                      </Button>
                    )}
                    <Button size="sm" variant="outline" className="w-full" onClick={() => void resetItem(item.id)}>
                      <RotateCcw className="h-4 w-4" />
                      <span>{t("wanted.reset")}</span>
                    </Button>
                    {item.mismatchRecoveryEligible ? (
                      <Button
                        size="sm"
                        variant="outline"
                        className="w-full"
                        onClick={() => void triggerMismatchRecovery(item.titleId)}
                      >
                        <RotateCcw className="h-4 w-4" />
                        <span>{t("wanted.actionRecoverMismatch")}</span>
                      </Button>
                    ) : null}
                  </div>
                  {expandedItemId === item.id ? (
                    <div className="mt-3 border-t border-border pt-3">
                      {decisionsLoading ? (
                        <p className="text-sm text-muted-foreground">{t("wanted.loadingDecisions")}</p>
                      ) : decisions.length === 0 ? (
                        <p className="text-sm text-muted-foreground">{t("wanted.noDecisions")}</p>
                      ) : (
                        <div className="space-y-2">
                          {decisions.map((d) => {
                            const scoringEntries = parseDecisionExplanation(d.explanationJson);
                            const hasScoringBreakdown = scoringEntries.length > 0;
                            const scoringExpanded = expandedDecisionIds.has(d.id);

                            return (
                              <div key={d.id} className="rounded-lg border border-border bg-background/40 p-3">
                                <p className="break-words text-xs font-medium text-foreground">{d.releaseTitle}</p>
                                <div className="mt-2 flex flex-wrap gap-2">
                                  {decisionBadge(d.decisionCode, t)}
                                  <span className="rounded bg-muted px-2 py-0.5 text-xs text-muted-foreground">
                                    {t("wanted.decScore")}: {d.candidateScore}
                                  </span>
                                  <span className="rounded bg-muted px-2 py-0.5 text-xs text-muted-foreground">
                                    {t("wanted.decDelta")}: {d.scoreDelta ?? "—"}
                                  </span>
                                </div>
                                <div className="mt-2 flex flex-wrap gap-3 text-xs text-muted-foreground">
                                  <span>{formatBytes(d.releaseSizeBytes)}</span>
                                  <span>{formatDate(d.createdAt, dateTimeFormat)}</span>
                                </div>
                                {hasScoringBreakdown ? (
                                  <div className="mt-3 border-t border-border pt-3">
                                    <button
                                      type="button"
                                      className="inline-flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
                                      onClick={() => toggleDecisionScoring(d.id)}
                                    >
                                      {scoringExpanded ? (
                                        <ChevronDown className="h-3.5 w-3.5" />
                                      ) : (
                                        <ChevronRight className="h-3.5 w-3.5" />
                                      )}
                                      <span>{t("wanted.scoreBreakdown")}</span>
                                    </button>
                                    {scoringExpanded ? (
                                      <ScoringBreakdown entries={scoringEntries} />
                                    ) : null}
                                  </div>
                                ) : null}
                              </div>
                            );
                          })}
                        </div>
                      )}
                    </div>
                  ) : null}
                </div>
              ))}
            </div>
          )
        ) : (
          <div
            className={
              shouldScrollDesktopTable
                ? "min-h-0 flex-1 overflow-auto rounded-[14px] border border-[var(--scry-border2)] bg-[var(--scry-surfC)]"
                : "overflow-x-auto rounded-[14px] border border-[var(--scry-border2)] bg-[var(--scry-surfC)]"
            }
          >
            <Table className="min-w-[980px]">
              <TableHeader
                className={shouldScrollDesktopTable ? "[&_th]:sticky [&_th]:top-0 [&_th]:z-10" : undefined}
              >
                <TableRow>
                  <TableHead className="w-8" />
                  <TableHead>{t("wanted.colTitle")}</TableHead>
                  <TableHead>Library</TableHead>
                  <TableHead>{t("wanted.colType")}</TableHead>
                  <TableHead>{t("wanted.colStatus")}</TableHead>
                  <TableHead>{t("wanted.colPhase")}</TableHead>
                  <TableHead>{t("wanted.colLatestDecision")}</TableHead>
                  <TableHead>{t("wanted.colNextSearch")}</TableHead>
                  <TableHead>{t("wanted.colScore")}</TableHead>
                  <TableHead>{t("wanted.colSearches")}</TableHead>
                  <TableHead>{t("label.actions")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {items.map((item) => (
                  <Fragment key={item.id}>
                    <TableRow className="group">
                      <TableCell>
                        <button
                          className="p-0.5 text-muted-foreground hover:text-foreground"
                          onClick={() => void loadDecisions(item.id)}
                        >
                          {expandedItemId === item.id ? (
                            <ChevronDown className="h-4 w-4" />
                          ) : (
                            <ChevronRight className="h-4 w-4" />
                          )}
                        </button>
                      </TableCell>
                      <TableCell className="max-w-[260px] text-sm" title={item.titleName ?? item.titleId}>
                        <button
                          type="button"
                          className="min-w-0 text-left hover:text-foreground"
                          onClick={() => openWantedItemOverview(item)}
                        >
                          <div className="truncate font-medium hover:underline">
                            {item.titleName ?? item.titleId.slice(0, 8)}
                          </div>
                          <div className="truncate text-xs text-muted-foreground">
                            {wantedItemSubtitle(item, t)}
                          </div>
                        </button>
                      </TableCell>
                      <TableCell className="max-w-[160px] text-xs text-muted-foreground">
                        <span className="block truncate">
                          {item.libraryName ?? item.libraryId ?? "Library"}
                        </span>
                      </TableCell>
                      <TableCell>{formatWantedMediaType(item.mediaType, t)}</TableCell>
                      <TableCell>{statusBadge(item.status, t)}</TableCell>
                      <TableCell>{phaseBadge(item.searchPhase, t)}</TableCell>
                      <TableCell className="text-xs">
                        {item.latestReleaseDecision ? (
                          <div className="space-y-1">
                            {decisionBadge(item.latestReleaseDecision.decisionCode, t)}
                            <div className="text-muted-foreground">
                              {formatDate(
                                item.latestReleaseDecision.createdAt,
                                dateTimeFormat,
                              )}
                            </div>
                          </div>
                        ) : (
                          "—"
                        )}
                      </TableCell>
                      <TableCell className="text-xs">
                        {formatDate(item.nextSearchAt, dateTimeFormat)}
                      </TableCell>
                      <TableCell>{item.currentScore ?? "—"}</TableCell>
                      <TableCell>{item.searchCount}</TableCell>
                      <TableCell>
                        <div className="flex gap-1">
                          <Button
                            size="icon"
                            variant="ghost"
                            className="h-7 w-7"
                            title={t("wanted.searchNow")}
                            onClick={() => void triggerSearch(item.id)}
                          >
                            <Search className="h-3.5 w-3.5" />
                          </Button>
                          {item.status === "paused" ? (
                            <Button
                              size="icon"
                              variant="ghost"
                              className="h-7 w-7"
                              title={t("wanted.resume")}
                              onClick={() => void resumeItem(item.id)}
                            >
                              <Play className="h-3.5 w-3.5" />
                            </Button>
                          ) : (
                            <Button
                              size="icon"
                              variant="ghost"
                              className="h-7 w-7"
                              title={t("wanted.pause")}
                              onClick={() => void pauseItem(item.id)}
                            >
                              <Pause className="h-3.5 w-3.5" />
                            </Button>
                          )}
                          <Button
                            size="icon"
                            variant="ghost"
                            className="h-7 w-7"
                            title={t("wanted.reset")}
                            onClick={() => void resetItem(item.id)}
                          >
                            <RotateCcw className="h-3.5 w-3.5" />
                          </Button>
                          {item.mismatchRecoveryEligible ? (
                            <Button
                              size="icon"
                              variant="ghost"
                              className="h-7 w-7"
                              title={t("wanted.actionRecoverMismatch")}
                              onClick={() => void triggerMismatchRecovery(item.titleId)}
                            >
                              <RefreshCw className="h-3.5 w-3.5" />
                            </Button>
                          ) : null}
                        </div>
                      </TableCell>
                    </TableRow>
                    {expandedItemId === item.id && (
                      <TableRow>
                        <TableCell colSpan={11} className="bg-muted/30 p-4">
                          {decisionsLoading ? (
                            <p className="text-sm text-muted-foreground">
                              {t("wanted.loadingDecisions")}
                            </p>
                          ) : decisions.length === 0 ? (
                            <p className="text-sm text-muted-foreground">
                              {t("wanted.noDecisions")}
                            </p>
                          ) : (
                            <Table className="min-w-[720px]">
                              <TableHeader>
                                <TableRow>
                                  <TableHead className="w-8" />
                                  <TableHead>{t("wanted.decRelease")}</TableHead>
                                  <TableHead>{t("wanted.decDecision")}</TableHead>
                                  <TableHead>{t("wanted.decScore")}</TableHead>
                                  <TableHead>{t("wanted.decDelta")}</TableHead>
                                  <TableHead>{t("wanted.decSize")}</TableHead>
                                  <TableHead>{t("wanted.decDate")}</TableHead>
                                </TableRow>
                              </TableHeader>
                              <TableBody>
                                {decisions.map((d) => {
                                  const scoringEntries = parseDecisionExplanation(d.explanationJson);
                                  const hasScoringBreakdown = scoringEntries.length > 0;
                                  const scoringExpanded = expandedDecisionIds.has(d.id);

                                  return (
                                    <Fragment key={d.id}>
                                      <TableRow>
                                        <TableCell>
                                          {hasScoringBreakdown ? (
                                            <button
                                              type="button"
                                              className="p-0.5 text-muted-foreground hover:text-foreground"
                                              onClick={() => toggleDecisionScoring(d.id)}
                                            >
                                              {scoringExpanded ? (
                                                <ChevronDown className="h-4 w-4" />
                                              ) : (
                                                <ChevronRight className="h-4 w-4" />
                                              )}
                                            </button>
                                          ) : null}
                                        </TableCell>
                                        <TableCell
                                          className="max-w-[300px] truncate text-xs"
                                          title={d.releaseTitle}
                                        >
                                          {d.releaseTitle}
                                        </TableCell>
                                        <TableCell>
                                          {decisionBadge(d.decisionCode, t)}
                                        </TableCell>
                                        <TableCell>{d.candidateScore}</TableCell>
                                        <TableCell>{d.scoreDelta ?? "—"}</TableCell>
                                        <TableCell className="text-xs">
                                          {formatBytes(d.releaseSizeBytes)}
                                        </TableCell>
                                        <TableCell className="text-xs">
                                          {formatDate(d.createdAt, dateTimeFormat)}
                                        </TableCell>
                                      </TableRow>
                                      {scoringExpanded ? (
                                        <TableRow>
                                          <TableCell colSpan={7} className="bg-background/70 p-3">
                                            <ScoringBreakdown entries={scoringEntries} />
                                          </TableCell>
                                        </TableRow>
                                      ) : null}
                                    </Fragment>
                                  );
                                })}
                              </TableBody>
                            </Table>
                          )}
                        </TableCell>
                      </TableRow>
                    )}
                  </Fragment>
                ))}
                {items.length === 0 && !loading && (
                  <TableRow>
                    <TableCell colSpan={10} className="text-center text-muted-foreground">
                      {t("wanted.noItems")}
                    </TableCell>
                  </TableRow>
                )}
              </TableBody>
            </Table>
          </div>
        )}

        {total > limit && (
          <div className="mt-4 flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
            <Button
              className="w-full sm:w-auto"
              size="sm"
              variant="outline"
              disabled={!hasPrev}
              onClick={() => setOffset(Math.max(0, offset - limit))}
            >
              {t("wanted.prev")}
            </Button>
            <span className="text-sm text-muted-foreground">
              {offset + 1}–{Math.min(offset + limit, total)} / {total}
            </span>
            <Button
              className="w-full sm:w-auto"
              size="sm"
              variant="outline"
              disabled={!hasNext}
              onClick={() => setOffset(offset + limit)}
            >
              {t("wanted.next")}
            </Button>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function ScoringBreakdown({
  entries,
}: {
  entries: ReleaseDecisionExplanationEntry[];
}) {
  const t = useTranslate();

  return (
    <div className="mt-3 rounded-md border border-border/70 bg-background/60 p-3">
      <div className="grid grid-cols-[minmax(0,1fr)_auto] gap-x-4 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
        <span>{t("wanted.scoreCode")}</span>
        <span>{t("wanted.decDelta")}</span>
      </div>
      <div className="mt-2 space-y-1">
        {entries.map((entry, index) => (
          <div
            key={`${entry.code}-${index}`}
            className="grid grid-cols-[minmax(0,1fr)_auto] gap-x-4 font-mono text-xs text-foreground"
          >
            <span className="truncate" title={entry.code}>
              {entry.code}
            </span>
            <span>{formatSignedDelta(entry.delta)}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

function formatTimeRemaining(delayUntil: string, t: Translate): string {
  const target = new Date(delayUntil).getTime();
  const now = Date.now();
  const diff = target - now;
  if (diff <= 0) return t("wanted.timeNow");
  const hours = Math.floor(diff / (1000 * 60 * 60));
  const minutes = Math.floor((diff % (1000 * 60 * 60)) / (1000 * 60));
  if (hours > 0) {
    return t("wanted.timeHoursMinutes", { hours, minutes });
  }
  return t("wanted.timeMinutes", { minutes });
}

function formatPendingStatus(status: PendingReleaseStatus): string {
  return status
    .split("_")
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

function pendingStatusBadge(status: PendingReleaseStatus) {
  const cls =
    status === "grabbed"
      ? "border-emerald-500/30 bg-emerald-500/15 text-emerald-300"
      : status === "expired" || status === "dismissed"
        ? "border-red-500/25 bg-red-500/12 text-red-300"
        : status === "processing"
          ? "border-sky-500/30 bg-sky-500/14 text-sky-300"
          : status === "superseded"
            ? "border-amber-500/30 bg-amber-500/14 text-amber-300"
            : "border-[var(--scry-border2)] bg-[var(--scry-chip)] text-[var(--scry-muted2)]";
  return (
    <span className={`inline-flex rounded-full border px-2 py-0.5 text-xs font-medium ${cls}`}>
      {formatPendingStatus(status)}
    </span>
  );
}

function pendingPhaseBadge(status: PendingReleaseStatus) {
  const label =
    status === "processing"
      ? "Processing"
      : status === "grabbed"
        ? "Grabbed"
        : status === "expired" || status === "dismissed"
          ? "Closed"
          : status === "superseded"
            ? "Superseded"
            : "Scheduled";
  return (
    <span className="inline-flex rounded-full border border-[rgba(var(--scry-accent-rgb),0.24)] bg-[rgba(var(--scry-accent-rgb),0.13)] px-2 py-0.5 text-xs font-medium text-[var(--scry-accent-text)]">
      {label}
    </span>
  );
}

function PendingReleasesCard({ state }: { state: PendingViewState }) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const isMobile = useIsMobile();
  const loadMoreRef = useRef<HTMLDivElement | null>(null);
  const {
    items,
    loading,
    hasMore,
    loadingMore,
    refreshItems,
    loadMoreItems,
    forceGrab,
    dismiss,
  } = state;
  const [expandedPendingId, setExpandedPendingId] = useState<string | null>(null);

  const togglePendingExpanded = (id: string) => {
    setExpandedPendingId((current) => (current === id ? null : id));
  };

  useEffect(() => {
    const node = loadMoreRef.current;
    if (!node || !hasMore || loadingMore) {
      return;
    }

    if (typeof IntersectionObserver === "undefined") {
      void loadMoreItems();
      return;
    }

    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          void loadMoreItems();
        }
      },
      { rootMargin: "900px 0px" },
    );
    observer.observe(node);
    return () => {
      observer.disconnect();
    };
  }, [hasMore, items.length, loadMoreItems, loadingMore]);

  return (
    <Card className="overflow-hidden rounded-none border-0 bg-transparent shadow-none">
      <CardHeader className="border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,var(--scry-surfD),transparent)] px-4 py-4 sm:px-5">
        <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <CardTitle className="text-[22px] font-bold tracking-normal text-[var(--scry-ink2)]">
            {t("pending.title")}
          </CardTitle>
          <Button
            className="h-10 w-full rounded-[10px] border-[var(--scry-border2)] bg-[var(--scry-inset)] px-3 text-[13px] text-[var(--scry-body)] shadow-none hover:bg-[var(--scry-hover)] sm:w-auto"
            size="sm"
            variant="secondary"
            onClick={() => void refreshItems()}
            disabled={loading}
          >
            <RefreshCw className="mr-1 h-3 w-3" />
            {loading ? t("wanted.refreshing") : t("label.refresh")}
          </Button>
        </div>
      </CardHeader>
      <CardContent className="bg-[color-mix(in_srgb,var(--scry-bg)_52%,transparent)] p-4 sm:p-5">
        {isMobile ? (
          items.length === 0 && !loading ? (
            <p className="text-center text-[var(--scry-muted3)]">{t("pending.noItems")}</p>
          ) : (
            <div className="space-y-3">
              {items.map((item) => {
                const expanded = expandedPendingId === item.id;
                return (
                <div key={item.id} className="rounded-[14px] border border-[var(--scry-border2)] bg-[var(--scry-surfC)] p-3 shadow-[0_12px_28px_rgba(2,6,23,0.10)]">
                  <div className="flex items-start gap-2">
                    <button
                      type="button"
                      className="mt-0.5 p-0.5 text-muted-foreground hover:text-foreground"
                      onClick={() => togglePendingExpanded(item.id)}
                    >
                      {expanded ? (
                        <ChevronDown className="h-4 w-4" />
                      ) : (
                        <ChevronRight className="h-4 w-4" />
                      )}
                    </button>
                    <div className="min-w-0 flex-1">
                      <p className="break-words text-sm font-medium text-foreground">{item.releaseTitle}</p>
                      <div className="mt-2 flex flex-wrap gap-2">
                        {pendingStatusBadge(item.status)}
                        {pendingPhaseBadge(item.status)}
                      </div>
                    </div>
                  </div>
                  <div className="mt-2 grid grid-cols-2 gap-2 text-xs text-muted-foreground">
                    <div>
                      <span className="block">{t("pending.colScore")}</span>
                      <span className="text-foreground">{item.releaseScore}</span>
                    </div>
                    <div>
                      <span className="block">{t("pending.colSize")}</span>
                      <span className="text-foreground">{item.releaseSizeBytes == null ? "—" : formatBytes(item.releaseSizeBytes)}</span>
                    </div>
                    <div>
                      <span className="block">{t("pending.colIndexer")}</span>
                      <span className="text-foreground">{item.indexerSource ?? "—"}</span>
                    </div>
                    <div>
                      <span className="block">{t("pending.colDelayUntil")}</span>
                      <span
                        className="text-foreground"
                        title={formatDate(item.delayUntil, dateTimeFormat)}
                      >
                        {formatTimeRemaining(item.delayUntil, t)}
                      </span>
                    </div>
                  </div>
                  <p className="mt-2 text-xs text-muted-foreground">
                    {formatDate(item.addedAt, dateTimeFormat)}
                  </p>
                  {expanded ? (
                    <div className="mt-3 grid gap-2 rounded-[10px] border border-[var(--scry-border3)] bg-[var(--scry-bg)] p-3 text-xs">
                      <div>
                        <span className="block text-muted-foreground">Title ID</span>
                        <span className="break-all text-foreground">{item.titleId}</span>
                      </div>
                      <div>
                        <span className="block text-muted-foreground">Wanted Item</span>
                        <span className="break-all text-foreground">{item.wantedItemId}</span>
                      </div>
                    </div>
                  ) : null}
                  <div className="mt-3 flex gap-2">
                    <Button size="sm" variant="secondary" className="flex-1" onClick={() => void forceGrab(item.id)}>
                      <Download className="h-4 w-4" />
                      <span>{t("pending.forceGrab")}</span>
                    </Button>
                    <Button size="sm" variant="outline" className="flex-1" onClick={() => void dismiss(item.id)}>
                      <X className="h-4 w-4" />
                      <span>{t("pending.dismiss")}</span>
                    </Button>
                  </div>
                </div>
                );
              })}
            </div>
          )
        ) : (
          <div className="overflow-auto rounded-[14px] border border-[var(--scry-border2)] bg-[var(--scry-surfC)]">
            <Table className="min-w-[980px]">
              <TableHeader>
                <TableRow>
                  <TableHead className="w-8" />
                  <TableHead>{t("pending.colRelease")}</TableHead>
                  <TableHead>{t("wanted.colStatus")}</TableHead>
                  <TableHead>{t("wanted.colPhase")}</TableHead>
                  <TableHead>{t("pending.colScore")}</TableHead>
                  <TableHead>{t("pending.colSize")}</TableHead>
                  <TableHead>{t("pending.colIndexer")}</TableHead>
                  <TableHead>{t("pending.colAddedAt")}</TableHead>
                  <TableHead>{t("pending.colDelayUntil")}</TableHead>
                  <TableHead>{t("label.actions")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {items.map((item) => {
                  const expanded = expandedPendingId === item.id;
                  return (
                    <Fragment key={item.id}>
                      <TableRow>
                        <TableCell>
                          <button
                            type="button"
                            className="p-0.5 text-muted-foreground hover:text-foreground"
                            onClick={() => togglePendingExpanded(item.id)}
                          >
                            {expanded ? (
                              <ChevronDown className="h-4 w-4" />
                            ) : (
                              <ChevronRight className="h-4 w-4" />
                            )}
                          </button>
                        </TableCell>
                        <TableCell className="max-w-[340px] truncate text-sm" title={item.releaseTitle}>
                          {item.releaseTitle}
                        </TableCell>
                        <TableCell>{pendingStatusBadge(item.status)}</TableCell>
                        <TableCell>{pendingPhaseBadge(item.status)}</TableCell>
                        <TableCell>{item.releaseScore}</TableCell>
                        <TableCell className="text-xs">
                          {item.releaseSizeBytes == null ? "—" : formatBytes(item.releaseSizeBytes)}
                        </TableCell>
                        <TableCell className="text-xs">{item.indexerSource ?? "—"}</TableCell>
                        <TableCell className="text-xs">
                          {formatDate(item.addedAt, dateTimeFormat)}
                        </TableCell>
                        <TableCell className="text-xs">
                          <span title={formatDate(item.delayUntil, dateTimeFormat)}>
                            {formatTimeRemaining(item.delayUntil, t)}
                          </span>
                        </TableCell>
                        <TableCell>
                          <div className="flex gap-1">
                            <Button
                              size="icon"
                              variant="ghost"
                              className="h-7 w-7"
                              title={t("pending.forceGrab")}
                              onClick={() => void forceGrab(item.id)}
                            >
                              <Download className="h-3.5 w-3.5" />
                            </Button>
                            <Button
                              size="icon"
                              variant="ghost"
                              className="h-7 w-7"
                              title={t("pending.dismiss")}
                              onClick={() => void dismiss(item.id)}
                            >
                              <X className="h-3.5 w-3.5" />
                            </Button>
                          </div>
                        </TableCell>
                      </TableRow>
                      {expanded ? (
                        <TableRow>
                          <TableCell colSpan={10} className="bg-background/30 p-4">
                            <div className="grid gap-3 text-xs sm:grid-cols-2 lg:grid-cols-4">
                              <div>
                                <span className="block text-muted-foreground">Title ID</span>
                                <span className="break-all text-foreground">{item.titleId}</span>
                              </div>
                              <div>
                                <span className="block text-muted-foreground">Wanted Item</span>
                                <span className="break-all text-foreground">{item.wantedItemId}</span>
                              </div>
                              <div>
                                <span className="block text-muted-foreground">{t("pending.colAddedAt")}</span>
                                <span className="text-foreground">{formatDate(item.addedAt, dateTimeFormat)}</span>
                              </div>
                              <div>
                                <span className="block text-muted-foreground">{t("pending.colDelayUntil")}</span>
                                <span className="text-foreground">{formatDate(item.delayUntil, dateTimeFormat)}</span>
                              </div>
                            </div>
                          </TableCell>
                        </TableRow>
                      ) : null}
                    </Fragment>
                  );
                })}
                {items.length === 0 && !loading && (
                  <TableRow>
                    <TableCell colSpan={10} className="text-center text-muted-foreground">
                      {t("pending.noItems")}
                    </TableCell>
                  </TableRow>
                )}
              </TableBody>
            </Table>
          </div>
        )}
        <div ref={loadMoreRef} aria-hidden="true" className="h-px" />
        {loadingMore ? (
          <p className="mt-3 text-center text-sm text-muted-foreground">
            {t("wanted.refreshing")}
          </p>
        ) : null}
      </CardContent>
    </Card>
  );
}
