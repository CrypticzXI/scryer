import * as React from "react";
import type { CSSProperties } from "react";
import type { LucideIcon } from "lucide-react";
import {
  Check,
  ChevronRight,
  Compass,
  Drama,
  Heart,
  Loader2,
  Palette,
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

function itemStableKey(item: DiscoveryItem) {
  return `${item.targetKind}:${item.targetKey}:${item.id}`;
}

function itemTypeLabel(item: DiscoveryItem) {
  const raw = item.contentType || item.targetKind;
  return raw.replace(/[_-]+/g, " ").trim().toUpperCase();
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
  return (
    formatScore(item.rankScore) ??
    (item.sourceCount ? `${item.sourceCount} sources` : null)
  );
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
        if (item.year != null && item.year < filters.minimumYear) {
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
  canManageTitle,
  canRequestMedia,
  onAction,
}: {
  item: DiscoveryItem;
  size?: "sm" | "md";
  canManageTitle: boolean;
  canRequestMedia: boolean;
  onAction: (item: DiscoveryItem) => void;
}) {
  const score = itemMatchScore(item);
  const compact = size === "sm";
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
            : "h-[225px] w-[152px] rounded-[13px]",
        )}
        style={posterFallbackStyle(item)}
      >
        <PosterImage item={item} />
        <div className="absolute inset-0 bg-gradient-to-b from-transparent via-transparent to-slate-950/90" />
        <span className="absolute left-2 top-2 rounded-[6px] bg-slate-950/70 px-2 py-0.5 text-[9.5px] font-bold tracking-[0.05em] text-slate-100 backdrop-blur">
          {itemTypeLabel(item)}
        </span>
        <div className="absolute right-2 top-2">
          <DiscoveryActionButton
            item={item}
            canManageTitle={canManageTitle}
            canRequestMedia={canRequestMedia}
            onAction={onAction}
            compact
          />
        </div>
        <div className="absolute bottom-8 left-2.5 right-2.5 font-[var(--font-space-grotesk)] text-[15px] font-bold leading-[1.05] text-white drop-shadow">
          {item.displayTitle}
        </div>
        <div className="absolute bottom-2.5 left-2.5 flex items-center gap-2 text-[11px] text-[var(--scry-text2)]">
          {item.year ? <span>{item.year}</span> : null}
          {score ? (
            <span className="inline-flex items-center gap-1 font-bold text-emerald-400">
              <TrendingUp className="h-3 w-3" />
              {score}
            </span>
          ) : null}
        </div>
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
}: {
  section: DiscoverySection;
  canManageTitle: boolean;
  canRequestMedia: boolean;
  onAction: (item: DiscoveryItem) => void;
  compact?: boolean;
}) {
  const t = useTranslate();

  return (
    <section className="mb-7">
      <div className="mb-3.5 flex items-center justify-between gap-3">
        <h3 className="m-0 font-[var(--font-space-grotesk)] text-lg font-semibold text-[var(--scry-ink2)]">
          {section.title}
        </h3>
        <button
          type="button"
          className="inline-flex items-center gap-1 text-[12.5px] font-medium text-[var(--scry-muted)]"
        >
          {t("discovery.viewAll")}
          <ChevronRight className="h-3.5 w-3.5" />
        </button>
      </div>
      <div className="flex gap-3.5 overflow-x-auto pb-1.5 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
        {section.items.map((item) => (
          <DiscoveryRailCard
            key={itemStableKey(item)}
            item={item}
            size={compact ? "sm" : "md"}
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
          {item.year ? <span className="font-semibold">{item.year}</span> : null}
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
        </div>
      </div>
    </section>
  );
}

function DiscoveryFilters({
  facets,
  items,
  selectedContentTypes,
  selectedGenre,
  selectedTag,
  minimumYear,
  minimumRating,
  onToggleContentType,
  onGenreChange,
  onTagChange,
  onMinimumYearChange,
  onMinimumRatingChange,
  onClear,
}: {
  facets: DiscoveryFacet[];
  items: DiscoveryItem[];
  selectedContentTypes: DiscoveryContentType[];
  selectedGenre: string;
  selectedTag: string;
  minimumYear: number;
  minimumRating: number;
  onToggleContentType: (contentType: DiscoveryContentType) => void;
  onGenreChange: (genre: string) => void;
  onTagChange: (tag: string) => void;
  onMinimumYearChange: (year: number) => void;
  onMinimumRatingChange: (rating: number) => void;
  onClear: () => void;
}) {
  const t = useTranslate();
  const contentTypes: Array<{
    key: DiscoveryContentType;
    label: string;
    count: number;
  }> = (
    [
    { key: "movie", label: t("discovery.type.movies") },
    { key: "series", label: t("discovery.type.series") },
    { key: "anime", label: t("discovery.type.anime") },
    ] as Array<{ key: DiscoveryContentType; label: string }>
  ).map((entry) => ({
    ...entry,
    count: facetCount(facets, entry.key) ?? contentTypeCount(items, entry.key),
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
  const minimumYearBound = years.length ? Math.min(...years) : 1900;
  const maximumYearBound = years.length ? Math.max(...years) : 2026;

  return (
    <aside className="w-[284px] flex-none overflow-y-auto border-l border-[var(--scry-border3)] bg-slate-950/25 px-5 py-5 max-xl:hidden">
      <div className="mb-4 flex items-center justify-between">
        <div className="flex items-center gap-2 font-[var(--font-space-grotesk)] text-[15px] font-semibold text-[var(--scry-ink2)]">
          <SlidersHorizontal className="h-4 w-4 text-[var(--scry-accent-text)]" />
          {t("discovery.filters")}
        </div>
        <button
          type="button"
          className="text-xs font-medium text-[var(--scry-accent-ring)]"
          onClick={onClear}
        >
          {t("discovery.clearAll")}
        </button>
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
      <select
        value={selectedGenre}
        onChange={(event) => onGenreChange(event.target.value)}
        className="mb-5 h-[38px] w-full rounded-[9px] border border-[var(--scry-border2)] bg-[var(--scry-bg)] px-3 text-[13px] text-[var(--scry-faint)] outline-none"
      >
        <option value="">{t("discovery.selectGenres")}</option>
        {genres.map((genre) => (
          <option key={genre} value={genre}>
            {genre}
          </option>
        ))}
      </select>
      <div className="mb-2.5 text-xs font-bold uppercase tracking-[0.05em] text-[var(--scry-muted2)]">
        {t("discovery.tags")}
      </div>
      <select
        value={selectedTag}
        onChange={(event) => onTagChange(event.target.value)}
        className="mb-3 h-[38px] w-full rounded-[9px] border border-[var(--scry-border2)] bg-[var(--scry-bg)] px-3 text-[13px] text-[var(--scry-faint)] outline-none"
      >
        <option value="">{t("discovery.selectTags")}</option>
        {tags.map((tag) => (
          <option key={tag} value={tag}>
            {tag}
          </option>
        ))}
      </select>
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
          {minimumYear} - {maximumYearBound}
        </span>
      </div>
      <input
        type="range"
        min={minimumYearBound}
        max={maximumYearBound}
        value={minimumYear}
        onChange={(event) => onMinimumYearChange(Number(event.target.value))}
        className="mb-6 w-full"
        style={{ accentColor: "var(--scry-accent)" }}
      />
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
        className="w-full"
        style={{ accentColor: "var(--scry-accent)" }}
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
  >(DISCOVERY_CONTENT_TYPES);
  const [selectedGenre, setSelectedGenre] = React.useState("");
  const [selectedTag, setSelectedTag] = React.useState("");
  const [minimumYear, setMinimumYear] = React.useState(1900);
  const [minimumRating, setMinimumRating] = React.useState(0);
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
  const effectiveMinimumYear = Math.min(
    Math.max(minimumYear, yearBounds.minimum),
    yearBounds.maximum,
  );
  const sections = React.useMemo(
    () =>
      filterDiscoverySections(rawSections, {
        contentTypes: selectedContentTypes,
        genre: selectedGenre,
        tag: selectedTag,
        minimumYear: effectiveMinimumYear,
        minimumRating,
      }),
    [
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
  const railSections = React.useMemo(
    () =>
      sections.filter(
        (section) => section.sectionId !== heroRailSection?.sectionId,
      ),
    [heroRailSection, sections],
  );
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
    setSelectedContentTypes(DISCOVERY_CONTENT_TYPES);
    setSelectedGenre("");
    setSelectedTag("");
    setMinimumYear(yearBounds.minimum);
    setMinimumRating(0);
  }, [yearBounds.minimum]);

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
        <div className="mb-5 flex items-center gap-1.5 border-b border-[var(--scry-border3)]">
          {TAB_DEFINITIONS.map((tab) => (
            <button
              key={tab.id}
              type="button"
              onClick={() => setActiveTab(tab.id)}
              className={cn(
                "relative px-3.5 py-2.5 text-[13.5px] font-semibold",
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
            {heroRailSection ? (
              <DiscoverySectionRail
                section={{
                  ...heroRailSection,
                  title:
                    heroRailSection.title || t("discovery.trendingThisWeek"),
                }}
                compact
                canManageTitle={canManageTitle}
                canRequestMedia={canRequestMedia}
                onAction={onAction}
              />
            ) : null}
          </div>
        ) : null}

        {railSections.length > 0 ? (
          railSections.map((section) => (
            <DiscoverySectionRail
              key={section.sectionId}
              section={section}
              canManageTitle={canManageTitle}
              canRequestMedia={canRequestMedia}
              onAction={onAction}
            />
          ))
        ) : !loading ? (
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
      </main>
      <DiscoveryFilters
        facets={home?.facets ?? []}
        items={rawItems}
        selectedContentTypes={selectedContentTypes}
        selectedGenre={selectedGenre}
        selectedTag={selectedTag}
        minimumYear={effectiveMinimumYear}
        minimumRating={minimumRating}
        onToggleContentType={toggleContentType}
        onGenreChange={setSelectedGenre}
        onTagChange={setSelectedTag}
        onMinimumYearChange={setMinimumYear}
        onMinimumRatingChange={setMinimumRating}
        onClear={clearFilters}
      />
    </div>
  );
}
