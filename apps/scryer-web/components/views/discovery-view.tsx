import * as React from "react";
import type { CSSProperties } from "react";
import type { LucideIcon } from "lucide-react";
import {
  Calendar,
  Check,
  ChevronDown,
  ChevronRight,
  Compass,
  Drama,
  Heart,
  Loader2,
  Palette,
  Play,
  Plus,
  Rocket,
  Scale,
  Send,
  Skull,
  SlidersHorizontal,
  Smile,
  Sparkles,
  Star,
  Swords,
  TrendingUp,
  Video,
  WandSparkles,
  X,
} from "lucide-react";
import { useTranslate } from "@/lib/context/translate-context";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type {
  DiscoveryFacet,
  DiscoveryHomePayload,
  DiscoveryItem,
  DiscoverySection,
} from "@/lib/types";

type DiscoveryViewProps = {
  home: DiscoveryHomePayload | null;
  loading: boolean;
  error: string | null;
  canManageTitle: boolean;
  canRequestMedia: boolean;
  onRefresh: () => void;
  onAction: (item: DiscoveryItem) => void;
};

type DiscoveryTabKey =
  | "forYou"
  | "trending"
  | "popular"
  | "upcoming"
  | "topRated"
  | "recentlyAdded";

type DiscoveryContentType = "movie" | "series" | "anime";

type GenreTile = {
  name: string;
  count: number;
  icon: LucideIcon;
  className: string;
};

const TAB_DEFINITIONS: Array<{ id: DiscoveryTabKey; labelKey: string }> = [
  { id: "forYou", labelKey: "discovery.tab.forYou" },
  { id: "trending", labelKey: "discovery.tab.trending" },
  { id: "popular", labelKey: "discovery.tab.popular" },
  { id: "upcoming", labelKey: "discovery.tab.upcoming" },
  { id: "topRated", labelKey: "discovery.tab.topRated" },
  { id: "recentlyAdded", labelKey: "discovery.tab.recentlyAdded" },
] as const;

const DISCOVERY_CONTENT_TYPES: DiscoveryContentType[] = [
  "movie",
  "series",
  "anime",
];
const DEFAULT_DISCOVERY_CONTENT_TYPES: DiscoveryContentType[] = [
  "movie",
  "series",
];
const DEFAULT_MINIMUM_YEAR = 1990;
const DEFAULT_MAXIMUM_YEAR = 2026;
const DEFAULT_MINIMUM_RATING = 7;

const GENRE_ICONS: LucideIcon[] = [
  Swords,
  Compass,
  Palette,
  Smile,
  Scale,
  Video,
  Drama,
  WandSparkles,
  Skull,
  Rocket,
];

const GENRE_TILE_CLASS_NAMES = [
  "from-rose-600 to-red-950",
  "from-orange-600 to-orange-950",
  "from-violet-600 to-violet-950",
  "from-yellow-600 to-yellow-950",
  "from-blue-600 to-blue-950",
  "from-emerald-600 to-emerald-950",
  "from-pink-600 to-pink-950",
  "from-teal-600 to-teal-950",
  "from-red-800 to-red-950",
  "from-cyan-700 to-cyan-950",
];
const FILTER_RANGE_CLASS_NAME =
  "h-1.5 w-full appearance-none rounded-full bg-[#16203a] accent-[var(--scry-accent)] [&::-moz-range-thumb]:h-[15px] [&::-moz-range-thumb]:w-[15px] [&::-moz-range-thumb]:rounded-full [&::-moz-range-thumb]:border-0 [&::-moz-range-thumb]:bg-white [&::-moz-range-thumb]:shadow-[0_1px_5px_rgba(0,0,0,0.5)] [&::-webkit-slider-runnable-track]:h-1.5 [&::-webkit-slider-runnable-track]:rounded-full [&::-webkit-slider-runnable-track]:bg-[#16203a] [&::-webkit-slider-thumb]:mt-[-4.5px] [&::-webkit-slider-thumb]:h-[15px] [&::-webkit-slider-thumb]:w-[15px] [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:bg-white [&::-webkit-slider-thumb]:shadow-[0_1px_5px_rgba(0,0,0,0.5)]";
const MONTH_ABBREVIATIONS = [
  "Jan",
  "Feb",
  "Mar",
  "Apr",
  "May",
  "Jun",
  "Jul",
  "Aug",
  "Sep",
  "Oct",
  "Nov",
  "Dec",
] as const;
const DATE_TOKEN_PATTERN = /(\d{4})[-/](\d{1,2})(?:[-/](\d{1,2}))?/;

function itemStableKey(item: DiscoveryItem) {
  return `${item.targetKind}:${item.targetKey}:${item.id}`;
}

function itemIdentityKey(item: DiscoveryItem) {
  return `${item.targetKind}:${item.targetKey}`;
}

function itemTypeLabel(item: DiscoveryItem) {
  const raw = item.contentType || item.targetKind;
  return raw.replace(/[_-]+/g, " ").trim().toUpperCase();
}

function formatCalendarDateToken(value: string) {
  const match = value.match(DATE_TOKEN_PATTERN);
  if (!match) {
    return null;
  }
  const month = Number(match[2]);
  if (!Number.isInteger(month) || month < 1 || month > 12) {
    return match[1];
  }
  const monthLabel = MONTH_ABBREVIATIONS[month - 1];
  const day = match[3] ? Number(match[3]) : null;
  return day && Number.isInteger(day) && day > 0
    ? `${monthLabel} ${day}`
    : `${monthLabel} ${match[1]}`;
}

function itemCalendarBadgeLabel(item: DiscoveryItem) {
  const dateLikeTag = [
    ...item.statusTags,
    ...item.contextTerms,
    ...item.sourceTags,
    ...item.relationSubtypes,
  ]
    .map(formatCalendarDateToken)
    .find((label): label is string => Boolean(label));
  return dateLikeTag ?? (item.year ? String(item.year) : null);
}

function hashHue(value: string) {
  let hash = 0;
  for (let index = 0; index < value.length; index += 1) {
    hash = (hash * 31 + value.charCodeAt(index)) % 360;
  }
  return hash;
}

function posterFallbackStyle(item: DiscoveryItem): CSSProperties {
  const hue = hashHue(item.targetKey || item.displayTitle);
  return {
    background: `radial-gradient(135% 100% at 50% 6%, hsl(${hue} 46% 33%) 0%, hsl(${(hue + 328) % 360} 52% 18%) 44%, #06080f 100%)`,
  };
}

function heroBackdropStyle(item: DiscoveryItem | null): CSSProperties {
  if (!item) {
    return {};
  }
  if (item.backgroundUrl) {
    return {
      backgroundImage: `url(${item.backgroundUrl})`,
    };
  }
  if (item.posterUrl) {
    return {
      backgroundImage: `url(${item.posterUrl})`,
    };
  }
  return posterFallbackStyle(item);
}

function formatScore(value: number | null | undefined) {
  if (value == null || Number.isNaN(value)) {
    return null;
  }
  if (value <= 1) {
    return `${Math.round(value * 100)}%`;
  }
  if (value <= 10) {
    return value.toFixed(1);
  }
  return `${Math.round(value)}%`;
}

function itemMatchScore(item: DiscoveryItem) {
  return formatScore(item.rankScore);
}

function allHomeSections(home: DiscoveryHomePayload | null) {
  if (!home) {
    return [];
  }
  return [
    ...home.publicSections,
    ...home.personalizedSections,
    ...(home.completeCollection ? [home.completeCollection] : []),
  ].filter((section) => section.items.length > 0);
}

function normalizedSectionText(section: DiscoverySection) {
  return `${section.sectionId} ${section.sectionType} ${section.title} ${section.surface}`.toLowerCase();
}

function sectionMatchesTab(section: DiscoverySection, activeTab: DiscoveryTabKey) {
  if (activeTab === "forYou") {
    return true;
  }
  const haystack = normalizedSectionText(section);
  if (activeTab === "trending") {
    return haystack.includes("trend");
  }
  if (activeTab === "popular") {
    return haystack.includes("popular");
  }
  if (activeTab === "upcoming") {
    return haystack.includes("upcoming") || haystack.includes("future");
  }
  if (activeTab === "topRated") {
    return haystack.includes("top") || haystack.includes("rated");
  }
  return haystack.includes("recent") || haystack.includes("added");
}

function sectionsForTab(
  home: DiscoveryHomePayload | null,
  activeTab: DiscoveryTabKey,
) {
  const sections = allHomeSections(home);
  if (activeTab === "forYou") {
    return [
      ...(home?.personalizedSections ?? []),
      ...sections.filter(
        (section) => !(home?.personalizedSections ?? []).includes(section),
      ),
    ].filter((section) => section.items.length > 0);
  }
  return sections.filter((section) => sectionMatchesTab(section, activeTab));
}

function sectionIsUpcoming(section: DiscoverySection) {
  const haystack = normalizedSectionText(section);
  return haystack.includes("upcoming") || haystack.includes("future");
}

function uniqueDiscoveryItems(items: DiscoveryItem[]) {
  const seen = new Set<string>();
  return items.filter((item) => {
    const key = itemIdentityKey(item);
    if (seen.has(key)) {
      return false;
    }
    seen.add(key);
    return true;
  });
}

function sectionWithoutItem(
  section: DiscoverySection | null,
  item: DiscoveryItem | null,
) {
  if (!section || !item) {
    return section;
  }
  const itemKey = itemIdentityKey(item);
  const items = section.items.filter(
    (candidate) => itemIdentityKey(candidate) !== itemKey,
  );
  return items.length > 0 ? { ...section, items } : null;
}

function firstHeroItem(sections: DiscoverySection[]) {
  return (
    sections
      .flatMap((section) => section.items)
      .find((item) => item.backgroundUrl || item.posterUrl || item.overview) ??
    sections[0]?.items[0] ??
    null
  );
}

function ratingForFilter(value: number | null | undefined) {
  if (value == null || Number.isNaN(value)) {
    return null;
  }
  return value <= 1 ? value * 10 : value;
}

function itemContentType(item: DiscoveryItem): DiscoveryContentType | null {
  const values = [item.targetKind, item.contentType ?? "", ...item.facetTerms].map(
    (value) => value.toLowerCase(),
  );
  if (values.some((value) => value.includes("anime"))) {
    return "anime";
  }
  if (values.some((value) => value.includes("series") || value.includes("show"))) {
    return "series";
  }
  if (values.some((value) => value.includes("movie") || value.includes("film"))) {
    return "movie";
  }
  return null;
}

function filterDiscoverySections(
  sections: DiscoverySection[],
  filters: {
    contentTypes: DiscoveryContentType[];
    genre: string;
    tag: string;
    minimumYear: number;
    maximumYear: number;
    minimumRating: number;
  },
) {
  return sections
    .map((section) => ({
      ...section,
      items: section.items.filter((item) => {
        const contentType = itemContentType(item);
        if (contentType && !filters.contentTypes.includes(contentType)) {
          return false;
        }
        if (
          filters.genre &&
          !item.genres.some(
            (genre) => genre.toLowerCase() === filters.genre.toLowerCase(),
          )
        ) {
          return false;
        }
        if (
          filters.tag &&
          ![...item.contextTerms, ...item.sourceTags, ...item.statusTags].some(
            (tag) => tag.toLowerCase() === filters.tag.toLowerCase(),
          )
        ) {
          return false;
        }
        if (
          item.year != null &&
          (item.year < filters.minimumYear || item.year > filters.maximumYear)
        ) {
          return false;
        }
        const rating = ratingForFilter(item.rating);
        if (rating != null && rating < filters.minimumRating) {
          return false;
        }
        return true;
      }),
    }))
    .filter((section) => section.items.length > 0);
}

function findHeroRailSection(sections: DiscoverySection[]) {
  return (
    sections.find((section) =>
      normalizedSectionText(section).includes("trend"),
    ) ??
    sections[0] ??
    null
  );
}

function buildGenreTiles(items: DiscoveryItem[]): GenreTile[] {
  const counts = new Map<string, number>();
  for (const item of items) {
    for (const genre of item.genres) {
      const label = genre.trim();
      if (label) {
        counts.set(label, (counts.get(label) ?? 0) + 1);
      }
    }
  }
  return [...counts.entries()]
    .sort((left, right) => right[1] - left[1] || left[0].localeCompare(right[0]))
    .slice(0, 10)
    .map(([name, count], index) => ({
      name,
      count,
      icon: GENRE_ICONS[index % GENRE_ICONS.length],
      className: GENRE_TILE_CLASS_NAMES[index % GENRE_TILE_CLASS_NAMES.length],
    }));
}

function contentTypeCount(items: DiscoveryItem[], kind: string) {
  const normalizedKind = kind.toLowerCase();
  return items.filter((item) => {
    const values = [
      item.targetKind,
      item.contentType ?? "",
      ...item.facetTerms,
    ].map((value) => value.toLowerCase());
    return values.some((value) => value.includes(normalizedKind));
  }).length;
}

function facetCount(facets: DiscoveryFacet[], value: string) {
  const normalizedValue = value.toLowerCase();
  return (
    facets.find((facet) => facet.value.toLowerCase() === normalizedValue)
      ?.smgCount ?? null
  );
}

function DiscoveryActionButton({
  item,
  canManageTitle,
  canRequestMedia,
  onAction,
  compact = false,
}: {
  item: DiscoveryItem;
  canManageTitle: boolean;
  canRequestMedia: boolean;
  onAction: (item: DiscoveryItem) => void;
  compact?: boolean;
}) {
  const t = useTranslate();
  const owned = item.ownedInInput;
  const Icon = owned ? Check : canManageTitle ? Plus : Send;
  const label = owned
    ? t("discovery.inLibrary")
    : canManageTitle
      ? t("discovery.add")
      : t("discovery.request");
  const disabled = owned || (!canManageTitle && !canRequestMedia);

  return (
    <button
      type="button"
      aria-label={`${label}: ${item.displayTitle}`}
      disabled={disabled}
      onClick={() => onAction(item)}
      className={cn(
        "inline-flex items-center justify-center gap-2 rounded-[10px] border border-white/20 bg-slate-950/75 text-white backdrop-blur transition hover:border-[var(--scry-accent)] hover:bg-[var(--scry-accent)] disabled:cursor-default disabled:border-white/10 disabled:bg-slate-950/45 disabled:text-white/60",
        compact ? "h-7 w-7" : "h-10 px-5 text-[13.5px] font-semibold",
      )}
    >
      <Icon className={compact ? "h-3.5 w-3.5" : "h-4 w-4"} />
      {compact ? null : <span>{label}</span>}
    </button>
  );
}

function PosterImage({ item }: { item: DiscoveryItem }) {
  return item.posterUrl ? (
    <img
      src={item.posterUrl}
      alt=""
      className="absolute inset-0 h-full w-full object-cover"
      loading="lazy"
    />
  ) : null;
}

function DiscoveryRailCard({
  item,
  size = "md",
  variant = "default",
  canManageTitle,
  canRequestMedia,
  onAction,
}: {
  item: DiscoveryItem;
  size?: "sm" | "md";
  variant?: "default" | "upcoming";
  canManageTitle: boolean;
  canRequestMedia: boolean;
  onAction: (item: DiscoveryItem) => void;
}) {
  const score = itemMatchScore(item);
  const compact = size === "sm";
  const upcoming = variant === "upcoming" && !compact;
  const calendarBadgeLabel = upcoming ? itemCalendarBadgeLabel(item) : null;
  return (
    <div
      className={cn(
        "group flex-none cursor-pointer transition-transform hover:-translate-y-1",
        compact ? "w-[120px]" : "w-[152px]",
      )}
    >
      <div
        className={cn(
          "relative overflow-hidden border border-[var(--scry-border2)] shadow-[0_10px_26px_rgba(0,0,0,0.35)]",
          compact
            ? "h-[178px] w-[120px] rounded-[11px]"
            : upcoming
              ? "h-[210px] w-[152px] rounded-[13px]"
              : "h-[225px] w-[152px] rounded-[13px]",
        )}
        style={posterFallbackStyle(item)}
      >
        <PosterImage item={item} />
        <div
          className={cn(
            "absolute inset-0",
            upcoming
              ? "bg-gradient-to-b from-slate-950/45 via-transparent to-slate-950/90"
              : "bg-gradient-to-b from-transparent via-transparent to-slate-950/90",
          )}
        />
        {calendarBadgeLabel ? (
          <span className="absolute left-2 top-2 inline-flex items-center gap-1 rounded-[7px] bg-slate-950/75 px-2.5 py-1 text-[9.5px] font-bold tracking-[0.04em] text-[#a9b3ff] backdrop-blur">
            <Calendar className="h-3 w-3" />
            {calendarBadgeLabel}
          </span>
        ) : (
          <span className="absolute left-2 top-2 rounded-[6px] bg-slate-950/70 px-2 py-0.5 text-[9.5px] font-bold tracking-[0.05em] text-slate-100 backdrop-blur">
            {itemTypeLabel(item)}
          </span>
        )}
        <div className="absolute right-2 top-2">
          <DiscoveryActionButton
            item={item}
            canManageTitle={canManageTitle}
            canRequestMedia={canRequestMedia}
            onAction={onAction}
            compact
          />
        </div>
        <div
          className={cn(
            "absolute left-2.5 right-2.5 font-[var(--font-space-grotesk)] text-[15px] font-bold leading-[1.05] text-white drop-shadow",
            upcoming ? "bottom-2.5" : "bottom-8",
          )}
        >
          <div>{item.displayTitle}</div>
          {upcoming ? (
            <div className="mt-1 font-sans text-[11px] font-medium text-[var(--scry-muted2)]">
              {itemTypeLabel(item)}
            </div>
          ) : null}
        </div>
        {upcoming ? null : (
          <div className="absolute bottom-2.5 left-2.5 flex items-center gap-2 text-[11px] text-[var(--scry-text2)]">
            {item.year ? <span>{item.year}</span> : null}
            {score ? (
              <span className="inline-flex items-center gap-1 font-bold text-emerald-400">
                <TrendingUp className="h-3 w-3" />
                {score}
              </span>
            ) : null}
          </div>
        )}
      </div>
      {compact ? (
        <>
          <div className="mt-2 truncate text-xs font-medium text-[var(--scry-body)]">
            {item.displayTitle}
          </div>
          <div className="flex items-center gap-1.5 text-[11px] text-[var(--scry-faint)]">
            {item.year ? <span>{item.year}</span> : null}
            {score ? (
              <span className="inline-flex items-center gap-1 font-bold text-emerald-400">
                <TrendingUp className="h-3 w-3" />
                {score}
              </span>
            ) : null}
          </div>
        </>
      ) : null}
    </div>
  );
}

function DiscoverySectionRail({
  section,
  canManageTitle,
  canRequestMedia,
  onAction,
  compact = false,
  variant = "default",
}: {
  section: DiscoverySection;
  canManageTitle: boolean;
  canRequestMedia: boolean;
  onAction: (item: DiscoveryItem) => void;
  compact?: boolean;
  variant?: "default" | "upcoming";
}) {
  const t = useTranslate();
  const items = React.useMemo(
    () => uniqueDiscoveryItems(section.items),
    [section.items],
  );

  return (
    <section className="mb-7">
      <div className="mb-3.5 flex items-center justify-between gap-3">
        <h3 className="m-0 font-[var(--font-space-grotesk)] text-lg font-semibold text-[var(--scry-ink2)]">
          {section.title}
        </h3>
        <span className="inline-flex items-center gap-1 text-[12.5px] font-medium text-[var(--scry-muted)]">
          {t("discovery.viewAll")}
          <ChevronRight className="h-3.5 w-3.5" />
        </span>
      </div>
      <div className="flex gap-3.5 overflow-x-auto pb-1.5 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
        {items.map((item) => (
          <DiscoveryRailCard
            key={itemStableKey(item)}
            item={item}
            size={compact ? "sm" : "md"}
            variant={variant}
            canManageTitle={canManageTitle}
            canRequestMedia={canRequestMedia}
            onAction={onAction}
          />
        ))}
      </div>
    </section>
  );
}

function DiscoveryHero({
  item,
  canManageTitle,
  canRequestMedia,
  onAction,
}: {
  item: DiscoveryItem;
  canManageTitle: boolean;
  canRequestMedia: boolean;
  onAction: (item: DiscoveryItem) => void;
}) {
  const t = useTranslate();
  const score = formatScore(item.rating);
  const match = itemMatchScore(item);
  const genres = item.genres.slice(0, 3);
  const statusLabel =
    item.statusTags
      .find((tag) => tag.trim().length > 0)
      ?.replace(/[_-]+/g, " ") ?? null;
  const sourceLabel =
    item.sourceCount && item.sourceCount > 0
      ? `${item.sourceCount} ${item.sourceCount === 1 ? "source" : "sources"}`
      : null;
  const detailItems = [
    item.year ? String(item.year) : null,
    statusLabel,
    sourceLabel,
  ].filter((detail): detail is string => Boolean(detail));
  return (
    <section className="relative min-h-[340px] overflow-hidden rounded-[18px] border border-[var(--scry-border2)] bg-slate-950">
      <div
        className="absolute inset-0 bg-cover bg-center opacity-90"
        style={heroBackdropStyle(item)}
      />
      <div className="absolute inset-0 bg-gradient-to-r from-slate-950 via-slate-950/75 to-slate-950/5" />
      <div className="absolute inset-0 bg-gradient-to-t from-slate-950/85 to-transparent" />
      <div className="relative flex h-full max-w-[min(62%,540px)] flex-col p-8 max-lg:max-w-[78%] max-sm:max-w-full">
        <div className="mb-3.5 flex flex-wrap gap-2">
          <span className="rounded-[7px] border border-[rgba(var(--scry-accent-rgb),0.4)] bg-[rgba(var(--scry-accent-rgb),0.22)] px-2.5 py-1 text-[11px] font-bold uppercase tracking-[0.04em] text-[#c3c9ff]">
            {t("discovery.featured")}
          </span>
          <span className="rounded-[7px] bg-white/10 px-2.5 py-1 text-[11px] font-semibold uppercase text-[#cfd7ee]">
            {itemTypeLabel(item)}
          </span>
        </div>
        <h2 className="m-0 mb-3 font-[var(--font-space-grotesk)] text-[clamp(2rem,4vw,42px)] font-bold leading-none text-white drop-shadow">
          {item.displayTitle}
        </h2>
        <div className="mb-3.5 flex flex-wrap items-center gap-3 text-[13px] text-[var(--scry-text2)]">
          {detailItems.map((detail, index) => (
            <React.Fragment key={`${detail}-${index}`}>
              {index > 0 ? (
                <span className="h-1 w-1 rounded-full bg-[var(--scry-faint2)]" />
              ) : null}
              <span className="font-semibold capitalize">{detail}</span>
            </React.Fragment>
          ))}
          {score ? (
            <span className="inline-flex items-center gap-1 rounded-[7px] bg-yellow-400/15 px-2 py-0.5 font-bold text-yellow-300">
              <Star className="h-3.5 w-3.5" />
              {score}
            </span>
          ) : null}
          {match ? (
            <span className="inline-flex items-center gap-1 rounded-[7px] bg-emerald-500/15 px-2 py-0.5 font-bold text-emerald-400">
              <Heart className="h-3.5 w-3.5" />
              {match}
            </span>
          ) : null}
        </div>
        {item.overview ? (
          <p className="m-0 mb-4 max-w-[430px] text-[13.5px] leading-6 text-[#b7c0dd]">
            {item.overview}
          </p>
        ) : null}
        <div className="mb-auto flex flex-wrap gap-2">
          {genres.map((genre) => (
            <span
              key={genre}
              className="rounded-[8px] border border-white/10 bg-white/10 px-3 py-1.5 text-xs text-[#cfd7ee]"
            >
              {genre}
            </span>
          ))}
        </div>
        <div className="mt-5 flex gap-3">
          <DiscoveryActionButton
            item={item}
            canManageTitle={canManageTitle}
            canRequestMedia={canRequestMedia}
            onAction={onAction}
          />
          <span className="inline-flex h-10 cursor-not-allowed items-center justify-center gap-2 rounded-[10px] border border-white/15 bg-white/10 px-4 text-[13.5px] font-semibold text-[var(--scry-ink3)] opacity-70 backdrop-blur">
            <Play className="h-4 w-4" aria-hidden="true" />
            <span>{t("discovery.trailer")}</span>
            <span className="sr-only">{t("discovery.trailerUnavailable")}</span>
          </span>
        </div>
      </div>
    </section>
  );
}

function DiscoveryFilters({
  variant = "desktop",
  facets,
  items,
  selectedContentTypes,
  selectedGenre,
  selectedTag,
  minimumYear,
  maximumYear,
  minimumRating,
  onToggleContentType,
  onGenreChange,
  onTagChange,
  onMinimumYearChange,
  onMaximumYearChange,
  onMinimumRatingChange,
  onClear,
  onRequestClose,
}: {
  variant?: "desktop" | "mobile";
  facets: DiscoveryFacet[];
  items: DiscoveryItem[];
  selectedContentTypes: DiscoveryContentType[];
  selectedGenre: string;
  selectedTag: string;
  minimumYear: number;
  maximumYear: number;
  minimumRating: number;
  onToggleContentType: (contentType: DiscoveryContentType) => void;
  onGenreChange: (genre: string) => void;
  onTagChange: (tag: string) => void;
  onMinimumYearChange: (year: number) => void;
  onMaximumYearChange: (year: number) => void;
  onMinimumRatingChange: (rating: number) => void;
  onClear: () => void;
  onRequestClose?: () => void;
}) {
  const t = useTranslate();
  const contentTypes: Array<{
    key: DiscoveryContentType;
    label: string;
    count: number;
  }> = DISCOVERY_CONTENT_TYPES.map((key) => ({
    key,
    label:
      key === "movie"
        ? t("discovery.type.movies")
        : key === "series"
          ? t("discovery.type.series")
          : t("discovery.type.anime"),
    count: facetCount(facets, key) ?? contentTypeCount(items, key),
  }));
  const genres = [...new Set(items.flatMap((item) => item.genres).filter(Boolean))]
    .sort((left, right) => left.localeCompare(right));
  const tags = [
    ...new Set(
      items
        .flatMap((item) => [
          ...item.contextTerms,
          ...item.sourceTags,
          ...item.statusTags,
        ])
        .filter(Boolean),
    ),
  ].sort((left, right) => left.localeCompare(right));
  const years = items
    .map((item) => item.year)
    .filter((year): year is number => typeof year === "number");
  const minimumYearBound = years.length ? Math.min(...years) : DEFAULT_MINIMUM_YEAR;
  const maximumYearBound = years.length ? Math.max(...years) : DEFAULT_MAXIMUM_YEAR;
  const yearSpan = Math.max(1, maximumYearBound - minimumYearBound);
  const minimumYearPercent =
    ((minimumYear - minimumYearBound) / yearSpan) * 100;
  const maximumYearPercent =
    ((maximumYear - minimumYearBound) / yearSpan) * 100;

  return (
    <aside
      className={cn(
        "overflow-y-auto px-5 py-5",
        variant === "desktop"
          ? "w-[284px] flex-none border-l border-[var(--scry-border3)] bg-slate-950/25 max-xl:hidden"
          : "h-full w-full border-l border-white/10 bg-slate-950/95 shadow-[-18px_0_38px_rgba(0,0,0,0.36)]",
      )}
    >
      <div className="mb-4 flex items-center justify-between">
        <div className="flex items-center gap-2 font-[var(--font-space-grotesk)] text-[15px] font-semibold text-[var(--scry-ink2)]">
          <SlidersHorizontal className="h-4 w-4 text-[var(--scry-accent-text)]" />
          {t("discovery.filters")}
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            className="text-xs font-medium text-[var(--scry-accent-ring)]"
            onClick={onClear}
          >
            {t("discovery.clearAll")}
          </button>
          {onRequestClose ? (
            <button
              type="button"
              aria-label={t("discovery.closeFilters")}
              onClick={onRequestClose}
              className="inline-flex h-8 w-8 items-center justify-center rounded-[8px] border border-[var(--scry-border2)] bg-[var(--scry-bg)] text-[var(--scry-muted)]"
            >
              <X className="h-4 w-4" />
            </button>
          ) : null}
        </div>
      </div>
      <div className="mb-2.5 text-xs font-bold uppercase tracking-[0.05em] text-[var(--scry-muted2)]">
        {t("discovery.contentType")}
      </div>
      <div className="mb-5 grid gap-2">
        {contentTypes.map((entry) => (
          <button
            key={entry.key}
            type="button"
            aria-pressed={selectedContentTypes.includes(entry.key)}
            onClick={() => onToggleContentType(entry.key)}
            className="flex items-center justify-between rounded-[9px] border border-[var(--scry-border2)] bg-[var(--scry-bg)] px-3 py-2 text-[13px] text-[var(--scry-text2)]"
          >
            <span className="inline-flex items-center gap-2">
              <span
                className={cn(
                  "flex h-[18px] w-[18px] items-center justify-center rounded-[5px] border",
                  selectedContentTypes.includes(entry.key)
                    ? "border-[var(--scry-accent)] bg-[var(--scry-accent)]"
                    : "border-[var(--scry-border2)] bg-transparent",
                )}
              >
                {selectedContentTypes.includes(entry.key) ? (
                  <Check className="h-3 w-3 text-white" />
                ) : null}
              </span>
              {entry.label}
            </span>
            <span className="text-[11px] text-[var(--scry-faint)]">
              {entry.count}
            </span>
          </button>
        ))}
      </div>
      <div className="mb-2.5 text-xs font-bold uppercase tracking-[0.05em] text-[var(--scry-muted2)]">
        {t("discovery.genres")}
      </div>
      <div className="relative mb-5">
        <select
          value={selectedGenre}
          onChange={(event) => onGenreChange(event.target.value)}
          className="h-[38px] w-full appearance-none rounded-[9px] border border-[var(--scry-border2)] bg-[var(--scry-bg)] px-3 pr-9 text-[13px] text-[var(--scry-faint)] outline-none transition hover:border-[var(--scry-bhover2)]"
        >
          <option value="">{t("discovery.selectGenres")}</option>
          {genres.map((genre) => (
            <option key={genre} value={genre}>
              {genre}
            </option>
          ))}
        </select>
        <ChevronDown className="pointer-events-none absolute right-3 top-1/2 h-[15px] w-[15px] -translate-y-1/2 text-[var(--scry-faint)]" />
      </div>
      <div className="mb-2.5 text-xs font-bold uppercase tracking-[0.05em] text-[var(--scry-muted2)]">
        {t("discovery.tags")}
      </div>
      <div className="relative mb-3">
        <select
          value={selectedTag}
          onChange={(event) => onTagChange(event.target.value)}
          className="h-[38px] w-full appearance-none rounded-[9px] border border-[var(--scry-border2)] bg-[var(--scry-bg)] px-3 pr-9 text-[13px] text-[var(--scry-faint)] outline-none transition hover:border-[var(--scry-bhover2)]"
        >
          <option value="">{t("discovery.selectTags")}</option>
          {tags.map((tag) => (
            <option key={tag} value={tag}>
              {tag}
            </option>
          ))}
        </select>
        <ChevronDown className="pointer-events-none absolute right-3 top-1/2 h-[15px] w-[15px] -translate-y-1/2 text-[var(--scry-faint)]" />
      </div>
      {selectedTag ? (
        <div className="mb-5 flex flex-wrap gap-2">
          <span className="rounded-[8px] border border-[rgba(var(--scry-accent-rgb),0.32)] bg-[rgba(var(--scry-accent-rgb),0.14)] px-3 py-1 text-xs font-semibold text-[var(--scry-accent-text)]">
            {selectedTag}
          </span>
        </div>
      ) : null}
      <div className="mb-2.5 flex items-center justify-between">
        <span className="text-xs font-bold uppercase tracking-[0.05em] text-[var(--scry-muted2)]">
          {t("discovery.releaseYear")}
        </span>
        <span className="text-[11.5px] text-[var(--scry-faint)]">
          {minimumYear} - {maximumYear}
        </span>
      </div>
      <div className="relative mb-6 h-5">
        <div className="absolute left-0 right-0 top-1/2 h-1.5 -translate-y-1/2 rounded-full bg-[#16203a]" />
        <div
          className="absolute top-1/2 h-1.5 -translate-y-1/2 rounded-full bg-gradient-to-r from-[var(--scry-accent)] to-[var(--scry-accent-ring)]"
          style={{
            left: `${minimumYearPercent}%`,
            right: `${100 - maximumYearPercent}%`,
          }}
        />
        <input
          type="range"
          min={minimumYearBound}
          max={maximumYearBound}
          value={minimumYear}
          aria-label={t("discovery.releaseYear")}
          onChange={(event) =>
            onMinimumYearChange(Math.min(Number(event.target.value), maximumYear))
          }
          className={cn(
            "absolute left-0 right-0 top-1/2 -translate-y-1/2 bg-transparent",
            FILTER_RANGE_CLASS_NAME,
          )}
        />
        <input
          type="range"
          min={minimumYearBound}
          max={maximumYearBound}
          value={maximumYear}
          aria-label={t("discovery.releaseYear")}
          onChange={(event) =>
            onMaximumYearChange(Math.max(Number(event.target.value), minimumYear))
          }
          className={cn(
            "absolute left-0 right-0 top-1/2 -translate-y-1/2 bg-transparent",
            FILTER_RANGE_CLASS_NAME,
          )}
        />
      </div>
      <div className="mb-2.5 flex items-center justify-between">
        <span className="text-xs font-bold uppercase tracking-[0.05em] text-[var(--scry-muted2)]">
          {t("discovery.minimumRating")}
        </span>
        <span className="text-[11.5px] font-bold text-[var(--scry-accent-ring)]">
          {minimumRating.toFixed(1)}+
        </span>
      </div>
      <input
        type="range"
        min={0}
        max={10}
        step={0.5}
        value={minimumRating}
        onChange={(event) => onMinimumRatingChange(Number(event.target.value))}
        className={FILTER_RANGE_CLASS_NAME}
      />
    </aside>
  );
}

export function DiscoveryView({
  home,
  loading,
  error,
  canManageTitle,
  canRequestMedia,
  onRefresh,
  onAction,
}: DiscoveryViewProps) {
  const t = useTranslate();
  const [activeTab, setActiveTab] = React.useState<DiscoveryTabKey>("forYou");
  const [selectedContentTypes, setSelectedContentTypes] = React.useState<
    DiscoveryContentType[]
  >(DEFAULT_DISCOVERY_CONTENT_TYPES);
  const [selectedGenre, setSelectedGenre] = React.useState("");
  const [selectedTag, setSelectedTag] = React.useState("");
  const [minimumYear, setMinimumYear] =
    React.useState(DEFAULT_MINIMUM_YEAR);
  const [maximumYear, setMaximumYear] =
    React.useState(DEFAULT_MAXIMUM_YEAR);
  const [minimumRating, setMinimumRating] = React.useState(
    DEFAULT_MINIMUM_RATING,
  );
  const [filtersOpen, setFiltersOpen] = React.useState(false);
  const rawSections = React.useMemo(
    () => sectionsForTab(home, activeTab),
    [activeTab, home],
  );
  const rawItems = React.useMemo(
    () => rawSections.flatMap((section) => section.items),
    [rawSections],
  );
  const yearBounds = React.useMemo(() => {
    const years = rawItems
      .map((item) => item.year)
      .filter((year): year is number => typeof year === "number");
    return {
      minimum: years.length ? Math.min(...years) : 1900,
      maximum: years.length ? Math.max(...years) : 2026,
    };
  }, [rawItems]);
  const effectiveMaximumYear = Math.max(
    Math.min(Math.max(maximumYear, yearBounds.minimum), yearBounds.maximum),
    yearBounds.minimum,
  );
  const effectiveMinimumYear = Math.min(
    Math.max(minimumYear, yearBounds.minimum),
    effectiveMaximumYear,
  );
  const sections = React.useMemo(
    () =>
      filterDiscoverySections(rawSections, {
        contentTypes: selectedContentTypes,
        genre: selectedGenre,
        tag: selectedTag,
        minimumYear: effectiveMinimumYear,
        maximumYear: effectiveMaximumYear,
        minimumRating,
      }),
    [
      effectiveMaximumYear,
      effectiveMinimumYear,
      minimumRating,
      rawSections,
      selectedContentTypes,
      selectedGenre,
      selectedTag,
    ],
  );
  const allItems = React.useMemo(
    () => sections.flatMap((section) => section.items),
    [sections],
  );
  const heroItem = React.useMemo(() => firstHeroItem(sections), [sections]);
  const genreTiles = React.useMemo(() => buildGenreTiles(allItems), [allItems]);
  const heroRailSection = React.useMemo(
    () => findHeroRailSection(sections),
    [sections],
  );
  const heroRailSectionWithoutHero = React.useMemo(
    () => sectionWithoutItem(heroRailSection, heroItem),
    [heroItem, heroRailSection],
  );
  const railSections = React.useMemo(
    () =>
      sections.filter(
        (section) => section.sectionId !== heroRailSection?.sectionId,
      ),
    [heroRailSection, sections],
  );
  const primaryRailSections = React.useMemo(
    () => railSections.filter((section) => !sectionIsUpcoming(section)),
    [railSections],
  );
  const upcomingRailSections = React.useMemo(
    () => railSections.filter(sectionIsUpcoming),
    [railSections],
  );
  const hasRenderableContent =
    heroItem !== null ||
    primaryRailSections.length > 0 ||
    upcomingRailSections.length > 0 ||
    genreTiles.length > 0;
  const toggleContentType = React.useCallback(
    (contentType: DiscoveryContentType) => {
      setSelectedContentTypes((current) =>
        current.includes(contentType)
          ? current.filter((item) => item !== contentType)
          : [...current, contentType],
      );
    },
    [],
  );
  const clearFilters = React.useCallback(() => {
    setSelectedContentTypes(DEFAULT_DISCOVERY_CONTENT_TYPES);
    setSelectedGenre("");
    setSelectedTag("");
    setMinimumYear(Math.max(yearBounds.minimum, DEFAULT_MINIMUM_YEAR));
    setMaximumYear(Math.min(yearBounds.maximum, DEFAULT_MAXIMUM_YEAR));
    setMinimumRating(DEFAULT_MINIMUM_RATING);
  }, [yearBounds.maximum, yearBounds.minimum]);
  React.useEffect(() => {
    if (!filtersOpen || typeof document === "undefined") {
      return undefined;
    }
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setFiltersOpen(false);
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.body.style.overflow = previousOverflow;
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [filtersOpen]);
  const filterProps = {
    facets: home?.facets ?? [],
    items: rawItems,
    selectedContentTypes,
    selectedGenre,
    selectedTag,
    minimumYear: effectiveMinimumYear,
    maximumYear: effectiveMaximumYear,
    minimumRating,
    onToggleContentType: toggleContentType,
    onGenreChange: setSelectedGenre,
    onTagChange: setSelectedTag,
    onMinimumYearChange: setMinimumYear,
    onMaximumYearChange: setMaximumYear,
    onMinimumRatingChange: setMinimumRating,
    onClear: clearFilters,
  };

  if (loading && !home) {
    return (
      <div className="flex min-h-[360px] items-center justify-center">
        <Loader2 className="h-6 w-6 animate-spin text-[var(--scry-accent)]" />
      </div>
    );
  }

  return (
    <div className="flex min-h-0 flex-1">
      <main className="min-w-0 flex-1 overflow-y-auto px-7 py-6 pb-16 max-sm:px-4">
        <div className="mb-5 flex items-center justify-between gap-3 border-b border-[var(--scry-border3)]">
          <div className="flex min-w-0 items-center gap-1.5 overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
            {TAB_DEFINITIONS.map((tab) => (
              <button
                key={tab.id}
                type="button"
                onClick={() => setActiveTab(tab.id)}
                className={cn(
                  "relative shrink-0 px-3.5 py-2.5 text-[13.5px] font-semibold",
                  activeTab === tab.id
                    ? "text-white"
                    : "text-[var(--scry-muted)] hover:text-[var(--scry-ink2)]",
                )}
              >
                {t(tab.labelKey)}
                {activeTab === tab.id ? (
                  <span className="absolute bottom-[-1px] left-2 right-2 h-[2.5px] rounded-full bg-[var(--scry-accent-ring)]" />
                ) : null}
              </button>
            ))}
          </div>
          <button
            type="button"
            aria-label={t("discovery.openFilters")}
            onClick={() => setFiltersOpen(true)}
            className="hidden h-9 shrink-0 items-center gap-2 rounded-[9px] border border-[var(--scry-border2)] bg-[var(--scry-bg)] px-3 text-[12.5px] font-semibold text-[var(--scry-ink2)] max-xl:inline-flex"
          >
            <SlidersHorizontal className="h-4 w-4 text-[var(--scry-accent-text)]" />
            {t("discovery.filters")}
          </button>
        </div>

        {error ? (
          <div className="mb-5 flex items-center justify-between gap-4 rounded-[12px] border border-rose-500/25 bg-rose-500/10 px-4 py-3 text-sm text-rose-200">
            <span>{error}</span>
            <Button type="button" size="sm" variant="outline" onClick={onRefresh}>
              {t("label.retry")}
            </Button>
          </div>
        ) : null}

        {heroItem ? (
          <div className="mb-7 grid grid-cols-[minmax(0,1.45fr)_minmax(0,1fr)] gap-5 max-lg:grid-cols-1">
            <DiscoveryHero
              item={heroItem}
              canManageTitle={canManageTitle}
              canRequestMedia={canRequestMedia}
              onAction={onAction}
            />
            {heroRailSectionWithoutHero ? (
              <DiscoverySectionRail
                section={{
                  ...heroRailSectionWithoutHero,
                  title:
                    heroRailSectionWithoutHero.title ||
                    t("discovery.trendingThisWeek"),
                }}
                compact
                canManageTitle={canManageTitle}
                canRequestMedia={canRequestMedia}
                onAction={onAction}
              />
            ) : null}
          </div>
        ) : null}

        {primaryRailSections.length > 0 ? (
          primaryRailSections.map((section) => (
            <DiscoverySectionRail
              key={section.sectionId}
              section={section}
              canManageTitle={canManageTitle}
              canRequestMedia={canRequestMedia}
              onAction={onAction}
            />
          ))
        ) : !loading && !hasRenderableContent ? (
          <div className="rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-surf)] px-5 py-8 text-center">
            <Sparkles className="mx-auto mb-3 h-6 w-6 text-[var(--scry-accent-text)]" />
            <h2 className="mb-2 font-[var(--font-space-grotesk)] text-lg font-semibold text-[var(--scry-ink2)]">
              {t("discovery.emptyTitle")}
            </h2>
            <p className="mx-auto max-w-md text-sm leading-6 text-[var(--scry-muted3)]">
              {t("discovery.emptyDescription")}
            </p>
          </div>
        ) : null}

        {genreTiles.length > 0 ? (
          <section className="mb-7">
            <h3 className="mb-3.5 font-[var(--font-space-grotesk)] text-lg font-semibold text-[var(--scry-ink2)]">
              {t("discovery.browseByGenre")}
            </h3>
            <div className="grid grid-cols-[repeat(auto-fit,minmax(150px,1fr))] gap-3">
              {genreTiles.map((genre) => {
                const Icon = genre.icon;
                return (
                  <button
                    key={genre.name}
                    type="button"
                    onClick={() => setSelectedGenre(genre.name)}
                    className={cn(
                      "flex h-[88px] flex-col justify-between overflow-hidden rounded-[13px] border border-white/10 bg-gradient-to-br p-3.5 text-left text-white transition hover:-translate-y-0.5 hover:border-white/30",
                      genre.className,
                    )}
                  >
                    <Icon className="h-[22px] w-[22px] drop-shadow" />
                    <div>
                      <div className="truncate text-sm font-bold drop-shadow">
                        {genre.name}
                      </div>
                      <div className="text-[11.5px] text-white/75">
                        {t("discovery.genreCount", { count: genre.count })}
                      </div>
                    </div>
                  </button>
                );
              })}
            </div>
          </section>
        ) : null}

        {upcomingRailSections.map((section) => (
          <DiscoverySectionRail
            key={section.sectionId}
            section={section}
            variant="upcoming"
            canManageTitle={canManageTitle}
            canRequestMedia={canRequestMedia}
            onAction={onAction}
          />
        ))}
      </main>
      {filtersOpen ? (
        <div className="fixed inset-0 z-50 xl:hidden">
          <button
            type="button"
            aria-label={t("discovery.closeFilters")}
            className="absolute inset-0 bg-slate-950/65 backdrop-blur-sm"
            onClick={() => setFiltersOpen(false)}
          />
          <div className="absolute bottom-0 right-0 top-0 w-[min(360px,100%)]">
            <DiscoveryFilters
              {...filterProps}
              variant="mobile"
              onRequestClose={() => setFiltersOpen(false)}
            />
          </div>
        </div>
      ) : null}
      <DiscoveryFilters {...filterProps} />
    </div>
  );
}
