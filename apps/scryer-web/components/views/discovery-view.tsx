import * as React from "react";
import type { CSSProperties } from "react";
import type { LucideIcon } from "lucide-react";
import {
  Check,
  ChevronRight,
  Clock,
  Disc3,
  Eye,
  Heart,
  Loader2,
  MonitorPlay,
  Plus,
  Send,
  SlidersHorizontal,
  Sparkles,
  X,
} from "lucide-react";
import { useTranslate } from "@/lib/context/translate-context";
import {
  AnidbExternalLink,
  AnilistExternalLink,
  ImdbExternalLink,
  MalExternalLink,
  TmdbExternalLink,
  TvdbMovieExternalLink,
  TvdbSeriesExternalLink,
} from "@/components/common/external-media-links";
import { Button } from "@/components/ui/button";
import { MultiSelectDropdown } from "@/components/ui/multi-select-dropdown";
import { TitleCard } from "@/components/title-card";
import type { TitleCardCornerBadge } from "@/components/title-card";
import { TitleRatingsStrip } from "@/components/views/title-ratings-strip";
import {
  canonicalDiscoveryFacetLabels,
  canonicalDiscoveryFilterOptions,
} from "@/lib/discovery-facets";
import { facetById } from "@/lib/facets/registry";
import {
  discoveryItemDisplayTitle,
  usefulDiscoveryTitle,
} from "@/lib/utils/discovery-display";
import {
  discoveryItemFacet,
  richExternalIdsFromDiscoverySignals,
} from "@/lib/utils/discovery-actions";
import { selectBackdropVariantUrl } from "@/lib/utils/poster-images";
import { cn } from "@/lib/utils";
import type {
  DiscoveryHomePayload,
  DiscoveryItem,
  DiscoverySection,
  DiscoverySyncStatus,
  Facet,
} from "@/lib/types";

type DiscoveryViewProps = {
  home: DiscoveryHomePayload | null;
  loading: boolean;
  error: string | null;
  manageableFacets: Facet[];
  requestableFacets: Facet[];
  onRefresh: () => void;
  onAction: (item: DiscoveryItem) => void;
};

type DiscoveryContentType = Facet;

const DISCOVERY_CONTENT_TYPES: DiscoveryContentType[] = [
  "MOVIE",
  "SERIES",
  "ANIME",
];
const DEFAULT_DISCOVERY_CONTENT_TYPES: DiscoveryContentType[] = [
  "MOVIE",
  "SERIES",
  "ANIME",
];
const DISCOVERY_FACET_PILL_CLASS: Record<DiscoveryContentType, string> = {
  MOVIE:
    "bg-[linear-gradient(135deg,rgba(var(--scry-facet-movie-rgb),0.96),rgba(var(--scry-facet-movie-rgb),0.72))] text-white",
  SERIES:
    "bg-[linear-gradient(135deg,rgba(var(--scry-facet-series-rgb),0.96),rgba(var(--scry-facet-series-rgb),0.72))] text-white",
  ANIME:
    "bg-[linear-gradient(135deg,rgba(var(--scry-facet-anime-rgb),0.96),rgba(var(--scry-facet-anime-rgb),0.72))] text-white",
};
const DEFAULT_MINIMUM_YEAR = 1900;
const DEFAULT_MINIMUM_RATING = 7;

function discoveryFacetIcon(
  facet: DiscoveryContentType | null | undefined,
): LucideIcon | null {
  return facet ? (facetById(facet)?.icon ?? null) : null;
}

const FILTER_RANGE_CLASS_NAME =
  "h-1.5 w-full appearance-none rounded-full bg-transparent accent-[var(--scry-accent)] [&::-moz-range-progress]:h-1.5 [&::-moz-range-progress]:rounded-full [&::-moz-range-progress]:bg-transparent [&::-moz-range-thumb]:h-[15px] [&::-moz-range-thumb]:w-[15px] [&::-moz-range-thumb]:rounded-full [&::-moz-range-thumb]:border-0 [&::-moz-range-thumb]:bg-white [&::-moz-range-thumb]:shadow-[0_1px_5px_rgba(0,0,0,0.5)] [&::-moz-range-track]:h-1.5 [&::-moz-range-track]:rounded-full [&::-moz-range-track]:bg-transparent [&::-webkit-slider-runnable-track]:h-1.5 [&::-webkit-slider-runnable-track]:rounded-full [&::-webkit-slider-runnable-track]:bg-transparent [&::-webkit-slider-thumb]:mt-[-4.5px] [&::-webkit-slider-thumb]:h-[15px] [&::-webkit-slider-thumb]:w-[15px] [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:bg-white [&::-webkit-slider-thumb]:shadow-[0_1px_5px_rgba(0,0,0,0.5)]";
const FILTER_RANGE_THUMB_POINTER_CLASS_NAME =
  "pointer-events-none [&::-moz-range-thumb]:pointer-events-auto [&::-webkit-slider-thumb]:pointer-events-auto";

function defaultMaximumDiscoveryYear() {
  return new Date().getFullYear() + 3;
}

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
  return `${item.targetKind}:${item.targetKey}`;
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
    ...(item.statusTags ?? []),
    ...(item.contextTerms ?? []),
    ...(item.sourceTags ?? []),
    ...(item.relationSubtypes ?? []),
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

function heroBackdropUrl(item: DiscoveryItem | null): string | null {
  return selectBackdropVariantUrl(item?.backgroundUrl ?? null, "w1280") ?? null;
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

const WEEKLY_FOR_YOU_SECTION_TYPES = [
  "TOP_MOVIES_THIS_WEEK",
  "TOP_SERIES_THIS_WEEK",
  "TOP_ANIME_THIS_WEEK",
];

const GENERIC_FOR_YOU_FALLBACK_SECTION_TYPES = new Set([
  "FOR_YOU",
  "MOVIES_FOR_YOU",
  "SERIES_FOR_YOU",
  "ANIME_FOR_YOU",
  "BECAUSE_YOU_HAVE",
]);

function discoverySectionType(section: DiscoverySection) {
  return section.sectionType.trim().toUpperCase();
}

// Public-promotion rails: SMG's v2 feed surfaces these two "new release window"
// sections. They arrive at feed-bottom with raw SMG titles; we lift them to just
// under the hero and give them curated (locale-owned) names + icons.
const NEW_ON_STREAMING_SECTION_TYPE = "NEW_ON_STREAMING";
const NEW_ON_PHYSICAL_SECTION_TYPE = "NEW_ON_PHYSICAL";
const PUBLIC_PROMOTION_SECTION_TYPES = [
  NEW_ON_STREAMING_SECTION_TYPE,
  NEW_ON_PHYSICAL_SECTION_TYPE,
] as const;
const PUBLIC_PROMOTION_SECTION_TYPE_SET = new Set<string>(
  PUBLIC_PROMOTION_SECTION_TYPES,
);

// sectionType -> i18n key. Preferred over SMG's raw section.title so product can
// re-word a rail purely in locale files. Unmapped types fall back to the raw title.
const SECTION_DISPLAY_NAME_KEYS: Record<string, string> = {
  [NEW_ON_STREAMING_SECTION_TYPE]: "discovery.section.newOnStreaming",
  [NEW_ON_PHYSICAL_SECTION_TYPE]: "discovery.section.newOnPhysical",
};

const SECTION_ICONS: Record<string, LucideIcon> = {
  [NEW_ON_STREAMING_SECTION_TYPE]: MonitorPlay,
  [NEW_ON_PHYSICAL_SECTION_TYPE]: Disc3,
};

function sectionDisplayTitle(
  section: DiscoverySection,
  t: ReturnType<typeof useTranslate>,
) {
  const key = SECTION_DISPLAY_NAME_KEYS[discoverySectionType(section)];
  if (key) {
    const label = t(key);
    if (label && label !== key) {
      return label;
    }
  }
  return section.title;
}

function sectionIcon(section: DiscoverySection): LucideIcon | null {
  return SECTION_ICONS[discoverySectionType(section)] ?? null;
}

function sectionIsPublicPromotion(section: DiscoverySection) {
  return PUBLIC_PROMOTION_SECTION_TYPE_SET.has(discoverySectionType(section));
}

function sectionIsCompleteCollection(section: DiscoverySection) {
  return (
    discoverySectionType(section) === "COMPLETE_THE_COLLECTION" ||
    section.sectionId === "complete_the_collection"
  );
}

// "More like this" recommendation strips — the rails where a franchise relation
// (sequel/spin-off/…) is the reason an item is being surfaced, so a relation pill
// adds context. Items without a known relation simply render no pill.
const MORE_LIKE_THIS_SECTION_TYPES = new Set([
  "BECAUSE_YOU_HAVE",
  "BECAUSE_YOU_LIKE_GENRE",
  "BECAUSE_YOU_LIKE_TAG",
]);

function sectionIsMoreLikeThis(section: DiscoverySection) {
  return MORE_LIKE_THIS_SECTION_TYPES.has(discoverySectionType(section));
}

// The seven SMG v2 relation types. Rendered nowhere before this program even
// though the values were already fetched/typed. Normalized then mapped to a
// locale key so wording stays in the locale files.
const RELATION_TYPE_LABEL_KEYS: Record<string, string> = {
  sequel: "discovery.relation.sequel",
  prequel: "discovery.relation.prequel",
  side_story: "discovery.relation.sideStory",
  spin_off: "discovery.relation.spinOff",
  adaptation: "discovery.relation.adaptation",
  shared_universe: "discovery.relation.sharedUniverse",
  alternative: "discovery.relation.alternative",
};

// Preference order when an item carries several relations — show the most
// "franchise-defining" one first.
const RELATION_TYPE_PRIORITY = [
  "sequel",
  "prequel",
  "spin_off",
  "side_story",
  "shared_universe",
  "adaptation",
  "alternative",
];

function normalizeRelationType(value: string): string | null {
  const normalized = value
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "");
  return normalized in RELATION_TYPE_LABEL_KEYS ? normalized : null;
}

function normalizedItemRelationTypes(item: DiscoveryItem): string[] {
  const seen = new Set<string>();
  const result: string[] = [];
  for (const raw of item.relationTypes ?? []) {
    const normalized = normalizeRelationType(raw);
    if (normalized && !seen.has(normalized)) {
      seen.add(normalized);
      result.push(normalized);
    }
  }
  return result;
}

function relationTypeLabel(
  relationType: string,
  t: ReturnType<typeof useTranslate>,
): string | null {
  const key = RELATION_TYPE_LABEL_KEYS[relationType];
  return key ? t(key) : null;
}

function primaryRelationType(item: DiscoveryItem): string | null {
  const relations = normalizedItemRelationTypes(item);
  if (relations.length === 0) {
    return null;
  }
  for (const candidate of RELATION_TYPE_PRIORITY) {
    if (relations.includes(candidate)) {
      return candidate;
    }
  }
  return relations[0];
}

function primaryRelationLabel(
  item: DiscoveryItem,
  t: ReturnType<typeof useTranslate>,
): string | null {
  const relationType = primaryRelationType(item);
  return relationType ? relationTypeLabel(relationType, t) : null;
}

// The distinct relation types actually present in the loaded feed, ordered by
// the display priority — drives the "Relationship" filter chip group so only
// meaningful chips appear (mirrors how genre/tag options are derived).
function presentRelationTypes(items: DiscoveryItem[]): string[] {
  const present = new Set<string>();
  for (const item of items) {
    for (const relationType of normalizedItemRelationTypes(item)) {
      present.add(relationType);
    }
  }
  return RELATION_TYPE_PRIORITY.filter((relationType) =>
    present.has(relationType),
  );
}

// --- Studio surfacing (SW3: studio_slug adoption) ---
// personIds also arrive on the item payload but are bare ids with no name
// source, so person filtering/labels are deferred to the P1 detail surface.

// Keep the Studio chip group scannable: the feed can span dozens of studios, so
// only the most frequent ones become chips (selected slugs always stay visible).
const STUDIO_FILTER_CHIP_LIMIT = 12;

function itemStudioSlug(item: DiscoveryItem): string | null {
  const slug = item.studioSlug?.trim().toLowerCase();
  return slug ? slug : null;
}

// Humanize a studio slug for display ("warner-bros-pictures" -> "Warner Bros
// Pictures"). Mirrors canonicalDiscoveryFacetLabel's word casing.
function studioSlugLabel(slug: string): string {
  return slug
    .split(/[-_:\s]+/)
    .filter(Boolean)
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1).toLowerCase())
    .join(" ");
}

function itemStudioLabel(item: DiscoveryItem): string | null {
  const slug = itemStudioSlug(item);
  return slug ? studioSlugLabel(slug) : null;
}

// The studio slugs present in the loaded feed, most frequent first (then
// alphabetical), capped so the chip group stays compact. Selected slugs are
// always included so an active filter can be toggled off even when its studio
// falls outside the cap.
function presentStudioSlugs(
  items: DiscoveryItem[],
  selectedSlugs: string[],
): string[] {
  const counts = new Map<string, number>();
  for (const item of items) {
    const slug = itemStudioSlug(item);
    if (slug) {
      counts.set(slug, (counts.get(slug) ?? 0) + 1);
    }
  }
  const ranked = [...counts.entries()]
    .sort(
      ([leftSlug, leftCount], [rightSlug, rightCount]) =>
        rightCount - leftCount || leftSlug.localeCompare(rightSlug),
    )
    .map(([slug]) => slug)
    .slice(0, STUDIO_FILTER_CHIP_LIMIT);
  const result = [...ranked];
  for (const slug of selectedSlugs) {
    if (!result.includes(slug)) {
      result.push(slug);
    }
  }
  return result;
}

function itemMatchesStudioSlugs(
  item: DiscoveryItem,
  selectedStudioSlugs: string[],
) {
  if (selectedStudioSlugs.length === 0) {
    return true;
  }
  const slug = itemStudioSlug(item);
  return slug !== null && selectedStudioSlugs.includes(slug);
}

function orderedHomeSections(home: DiscoveryHomePayload | null) {
  const sections = allHomeSections(home);
  const personalizedSections = (home?.personalizedSections ?? []).filter(
    (section) => section.items.length > 0,
  );
  const completeCollection =
    home?.completeCollection && home.completeCollection.items.length > 0
      ? home.completeCollection
      : null;
  const usedPersonalizedSections = new Set<DiscoverySection>();
  const takePersonalizedSections = (sectionType: string) =>
    personalizedSections.filter((section) => {
      if (discoverySectionType(section) !== sectionType) {
        return false;
      }
      usedPersonalizedSections.add(section);
      return true;
    });
  const promotedSections = [
    ...WEEKLY_FOR_YOU_SECTION_TYPES.flatMap(takePersonalizedSections),
    ...takePersonalizedSections("BECAUSE_YOU_LIKE_GENRE"),
    ...takePersonalizedSections("BECAUSE_YOU_LIKE_TAG"),
    ...takePersonalizedSections("TOP_RATED_ACCLAIMED_NOT_IN_LIBRARY"),
  ];
  const unknownPersonalizedSections = personalizedSections.filter(
    (section) =>
      !usedPersonalizedSections.has(section) &&
      !GENERIC_FOR_YOU_FALLBACK_SECTION_TYPES.has(discoverySectionType(section)),
  );
  const hasLibrarySections =
    promotedSections.length > 0 ||
    completeCollection !== null ||
    unknownPersonalizedSections.length > 0;
  const fallbackSections = hasLibrarySections
    ? []
    : personalizedSections.filter((section) =>
        GENERIC_FOR_YOU_FALLBACK_SECTION_TYPES.has(discoverySectionType(section)),
      );
  // Public-promotion tier: the two SMG v2 "new release window" rails, lifted from
  // feed-bottom to directly under the hero row, in a fixed streaming-then-physical
  // order regardless of where SMG placed them in the public feed.
  const publicSections = (home?.publicSections ?? []).filter(
    (section) => section.items.length > 0,
  );
  const publicPromotionSections = PUBLIC_PROMOTION_SECTION_TYPES.flatMap(
    (sectionType) =>
      publicSections.filter(
        (section) => discoverySectionType(section) === sectionType,
      ),
  );
  const orderedSections = [
    ...promotedSections,
    ...(completeCollection ? [completeCollection] : []),
    ...unknownPersonalizedSections,
    ...fallbackSections,
  ];
  const orderedSectionSet = new Set([
    ...orderedSections,
    ...publicPromotionSections,
  ]);
  return [
    ...publicPromotionSections,
    ...orderedSections,
    ...sections.filter(
      (section) =>
        !orderedSectionSet.has(section) &&
        !personalizedSections.includes(section) &&
        section !== completeCollection,
    ),
  ].filter((section) => section.items.length > 0);
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
      .find((item) => item.backgroundUrl) ??
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

function matchesAnySelectedValue(values: string[], selectedValues: string[]) {
  if (selectedValues.length === 0) {
    return true;
  }
  const normalizedValues = new Set(
    values.map((value) => value.trim().toLowerCase()),
  );
  return selectedValues.some((value) =>
    normalizedValues.has(value.trim().toLowerCase()),
  );
}

function normalizedDiscoveryContentType(
  value: string | null | undefined,
): DiscoveryContentType | null {
  switch (value?.trim().toLowerCase()) {
    case "anime":
      return "ANIME";
    case "series":
      return "SERIES";
    case "movie":
      return "MOVIE";
    default:
      return null;
  }
}

function itemContentType(item: DiscoveryItem): DiscoveryContentType | null {
  const contentType = item.contentType?.trim();
  return contentType
    ? normalizedDiscoveryContentType(contentType)
    : normalizedDiscoveryContentType(item.targetKind);
}

function discoveryExternalIdMap(item: DiscoveryItem) {
  const ids = richExternalIdsFromDiscoverySignals(item)
    .map((externalId) => ({
      source: externalId.source.trim().toLowerCase(),
      value: externalId.value.trim(),
      kind: externalId.kind?.trim().toLowerCase() || null,
    }))
    .filter((externalId) => externalId.source && externalId.value);
  const bySource = (source: string) =>
    ids.find((externalId) => externalId.source === source)?.value;
  const bySourceKind = (source: string, kind: string) =>
    ids.find(
      (externalId) =>
        externalId.source === source &&
        (externalId.kind === kind ||
          (kind === "tv" && externalId.kind === "series") ||
          (kind === "series" && externalId.kind === "tv")),
    )?.value ?? bySource(source);
  return {
    bySource,
    bySourceKind,
    has: (source: string) => Boolean(bySource(source)),
  };
}

function discoveryItemDisplayGenreLabels(item: DiscoveryItem): string[] {
  return canonicalDiscoveryFacetLabels(item, "genre");
}

type DiscoveryItemFilters = {
  contentTypes: DiscoveryContentType[];
  genres: string[];
  tags: string[];
  relationTypes: string[];
  studioSlugs: string[];
  minimumYear: number;
  maximumYear: number;
  minimumRating: number;
};

function itemMatchesRelationTypes(
  item: DiscoveryItem,
  selectedRelationTypes: string[],
) {
  if (selectedRelationTypes.length === 0) {
    return true;
  }
  const relations = new Set(normalizedItemRelationTypes(item));
  return selectedRelationTypes.some((relationType) =>
    relations.has(relationType),
  );
}

function itemMatchesDiscoveryFacetFilters(
  item: DiscoveryItem,
  filters: Omit<DiscoveryItemFilters, "contentTypes">,
) {
  if (
    !matchesAnySelectedValue(
      canonicalDiscoveryFacetLabels(item, "genre"),
      filters.genres,
    )
  ) {
    return false;
  }
  if (
    !matchesAnySelectedValue(canonicalDiscoveryFacetLabels(item, "theme"), filters.tags)
  ) {
    return false;
  }
  if (!itemMatchesRelationTypes(item, filters.relationTypes)) {
    return false;
  }
  if (!itemMatchesStudioSlugs(item, filters.studioSlugs)) {
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
}

function filterDiscoverySections(
  sections: DiscoverySection[],
  filters: DiscoveryItemFilters,
) {
  return sections
    .map((section) => ({
      ...section,
      items: section.items.filter((item) => {
        if (!discoveryItemHasUsefulTitle(item)) {
          return false;
        }
        const contentType = itemContentType(item);
        if (contentType && !filters.contentTypes.includes(contentType)) {
          return false;
        }
        return itemMatchesDiscoveryFacetFilters(item, filters);
      }),
    }))
    .filter((section) => section.items.length > 0);
}

function discoveryItemHasUsefulTitle(item: DiscoveryItem) {
  return Boolean(
    usefulDiscoveryTitle(item.displayTitle) ||
      usefulDiscoveryTitle(item.sortTitle) ||
      usefulDiscoveryTitle(item.originalTitle),
  );
}

function findHeroRailSection(sections: DiscoverySection[]) {
  // Never fold a public-promotion rail into the hero column — those stay as full
  // rails directly beneath the hero.
  const eligible = sections.filter(
    (section) => !sectionIsPublicPromotion(section),
  );
  return (
    eligible.find((section) =>
      normalizedSectionText(section).includes("trend"),
    ) ??
    eligible[0] ??
    null
  );
}

function contentTypeCount(items: DiscoveryItem[], kind: string) {
  return items.filter((item) => itemContentType(item) === kind).length;
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
  const titleLabel = discoveryItemDisplayTitle(item);

  if (compact) {
    return (
      <button
        type="button"
        aria-label={`${label}: ${titleLabel}`}
        disabled={disabled}
        onClick={() => onAction(item)}
        className="inline-flex h-7 w-7 items-center justify-center gap-2 rounded-[10px] border border-white/20 bg-slate-950/75 text-white backdrop-blur transition hover:border-[var(--scry-accent)] hover:bg-[var(--scry-accent)] disabled:cursor-default disabled:border-white/10 disabled:bg-slate-950/45 disabled:text-white/60"
      >
        <Icon className="h-3.5 w-3.5" />
      </button>
    );
  }

  return (
    <Button
      type="button"
      variant="primary"
      size="lg"
      aria-label={`${label}: ${titleLabel}`}
      disabled={disabled}
      onClick={() => onAction(item)}
      className="h-12 w-96 max-w-full rounded-[12px] px-7 text-[15px] font-semibold shadow-[0_16px_30px_rgba(var(--scry-accent-rgb),0.22)]"
    >
      <Icon className="h-5 w-5" />
      <span>{label}</span>
    </Button>
  );
}

function DiscoveryRailCard({
  item,
  size = "md",
  variant = "default",
  fillHeight = false,
  canManageTitle,
  canRequestMedia,
  onAction,
  cornerBadge,
  onDismiss,
  dismissLabel,
}: {
  item: DiscoveryItem;
  size?: "sm" | "md";
  variant?: "default" | "upcoming";
  fillHeight?: boolean;
  canManageTitle: boolean;
  canRequestMedia: boolean;
  onAction: (item: DiscoveryItem) => void;
  cornerBadge?: TitleCardCornerBadge | null;
  onDismiss?: (item: DiscoveryItem) => void;
  dismissLabel?: string;
}) {
  const compactSize = size === "sm";
  const upcoming = variant === "upcoming" && !compactSize;
  const owned = item.ownedInInput;
  const addable = !owned && canManageTitle;
  const requestable = !owned && !canManageTitle && canRequestMedia;
  const facet = itemContentType(item);
  const subtitle = upcoming ? itemCalendarBadgeLabel(item) : item.year;
  const handleAction = React.useCallback(
    () => onAction(item),
    [onAction, item],
  );
  const handleDismiss = React.useMemo(
    () => (onDismiss ? () => onDismiss(item) : undefined),
    [onDismiss, item],
  );
  return (
    <div
      className={cn(
        "flex-none",
        fillHeight
          ? "aspect-[2/3] h-full"
          : compactSize
            ? "w-[120px]"
            : "w-[152px]",
      )}
    >
      <TitleCard
        title={discoveryItemDisplayTitle(item)}
        year={subtitle}
        facet={facet}
        facetLabel={itemTypeLabel(item)}
        posterUrl={item.posterUrl}
        addable={addable}
        requestable={requestable}
        compact={!fillHeight}
        cornerBadge={cornerBadge}
        onDismiss={handleDismiss}
        dismissLabel={dismissLabel}
        onAdd={addable ? handleAction : undefined}
        onRequest={requestable ? handleAction : undefined}
      />
    </div>
  );
}

function DiscoverySectionRail({
  section,
  manageableFacets,
  requestableFacets,
  onAction,
  onDismissItem,
  compact = false,
  fillHeight = false,
  variant = "default",
}: {
  section: DiscoverySection;
  manageableFacets: ReadonlySet<Facet>;
  requestableFacets: ReadonlySet<Facet>;
  onAction: (item: DiscoveryItem) => void;
  onDismissItem?: (item: DiscoveryItem) => void;
  compact?: boolean;
  fillHeight?: boolean;
  variant?: "default" | "upcoming";
}) {
  const t = useTranslate();
  const items = React.useMemo(
    () => uniqueDiscoveryItems(section.items),
    [section.items],
  );
  // Surface the relation pill where the relationship is the point of the rail.
  const relationRail =
    sectionIsCompleteCollection(section) || sectionIsMoreLikeThis(section);
  const HeaderIcon = sectionIcon(section);
  const heading = sectionDisplayTitle(section, t);
  const dismissLabel = t("discovery.notInterested");

  const cornerBadgeFor = React.useCallback(
    (item: DiscoveryItem): TitleCardCornerBadge | null => {
      if (relationRail) {
        const relationLabel = primaryRelationLabel(item, t);
        if (relationLabel) {
          return { label: relationLabel, tone: "accent" };
        }
        // No franchise relation: fall back to studio provenance — the
        // next-best "why this recommendation" context. Kept to relation rails
        // so generic rails stay uncluttered (most items carry a studio).
        const studioLabel = itemStudioLabel(item);
        if (studioLabel) {
          return {
            label: studioLabel,
            tone: "neutral",
            title: `${t("discovery.studio")}: ${studioLabel}`,
          };
        }
      }
      return null;
    },
    [relationRail, t],
  );

  return (
    <section className={cn("mb-7", fillHeight && "flex h-full min-h-0 flex-col")}>
      <div className="mb-3.5 flex items-center justify-between gap-3">
        <h3 className="m-0 inline-flex items-center gap-2 font-[var(--font-space-grotesk)] text-lg font-semibold text-[var(--scry-ink2)]">
          {HeaderIcon ? (
            <HeaderIcon
              className="h-4 w-4 text-[var(--scry-accent-text)]"
              aria-hidden="true"
            />
          ) : null}
          {heading}
        </h3>
        <span className="inline-flex items-center gap-1 text-[12.5px] font-medium text-[var(--scry-muted)]">
          {t("discovery.viewAll")}
          <ChevronRight className="h-3.5 w-3.5" />
        </span>
      </div>
      <div
        className={cn(
          "flex gap-3.5 overflow-x-auto pb-1.5 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden",
          fillHeight && "min-h-0 flex-1",
        )}
      >
        {items.map((item) => {
          const facet = discoveryItemFacet(item);
          return (
            <DiscoveryRailCard
              key={itemStableKey(item)}
              item={item}
              size={compact ? "sm" : "md"}
              variant={variant}
              fillHeight={fillHeight}
              canManageTitle={facet !== null && manageableFacets.has(facet)}
              canRequestMedia={facet !== null && requestableFacets.has(facet)}
              onAction={onAction}
              cornerBadge={cornerBadgeFor(item)}
              onDismiss={onDismissItem}
              dismissLabel={dismissLabel}
            />
          );
        })}
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
  const titleLabel = discoveryItemDisplayTitle(item);
  const match = itemMatchScore(item);
  const genres = discoveryItemDisplayGenreLabels(item).slice(0, 3);
  const facet = itemContentType(item);
  const FacetIcon = discoveryFacetIcon(facet);
  const externalIds = discoveryExternalIdMap(item);
  const hasExternalLinks = [
    "imdb",
    "tmdb",
    "tvdb",
    "mal",
    "anilist",
    "anidb",
  ].some((source) => externalIds.has(source));
  const tmdbMediaType = facet === "MOVIE" ? "movie" : "tv";
  const tvdbKind = facet === "MOVIE" ? "movie" : "series";
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
  const backdropUrl = heroBackdropUrl(item);
  return (
    <section className="relative min-h-[340px] overflow-hidden rounded-[18px] border border-[var(--scry-border2)] bg-slate-950">
      {backdropUrl ? (
        <img
          src={backdropUrl}
          alt=""
          aria-hidden="true"
          data-discovery-hero-backdrop="true"
          className="absolute inset-0 h-full w-full object-cover"
        />
      ) : (
        <div
          className="absolute inset-0"
          style={posterFallbackStyle(item)}
          data-discovery-hero-backdrop-fallback="true"
        />
      )}
      <div className="absolute inset-0 bg-gradient-to-r from-slate-950/80 via-slate-950/45 to-slate-950/0" />
      <div className="absolute inset-0 bg-gradient-to-t from-slate-950/55 via-slate-950/15 to-transparent" />
      <div className="relative flex min-h-[340px] flex-col p-8 pb-28 max-sm:pb-8">
        <div className="max-w-[min(72%,760px)] max-lg:max-w-[82%] max-sm:max-w-full">
          <div className="mb-3.5 flex flex-wrap gap-2">
            <span className="rounded-[7px] border border-[rgba(var(--scry-accent-rgb),0.4)] bg-[rgba(var(--scry-accent-rgb),0.22)] px-2.5 py-1 text-[11px] font-bold uppercase tracking-[0.04em] text-[#c3c9ff]">
              {t("discovery.featured")}
            </span>
            <span
              className={cn(
                "inline-flex items-center gap-1.5 rounded-[8px] px-2.5 py-1 text-[11px] font-black uppercase tracking-[0.035em] shadow-[inset_0_1px_0_rgba(255,255,255,0.28),0_8px_18px_rgba(0,0,0,0.22)]",
                facet
                  ? DISCOVERY_FACET_PILL_CLASS[facet]
                  : "bg-white/15 text-[#cfd7ee]",
              )}
            >
              {FacetIcon ? (
                <FacetIcon className="h-3.5 w-3.5" aria-hidden="true" />
              ) : null}
              {itemTypeLabel(item)}
            </span>
          </div>
          <h2 className="m-0 mb-3 font-[var(--font-space-grotesk)] text-[clamp(2rem,3.5vw,46px)] font-bold leading-none text-white drop-shadow">
            {titleLabel}
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
            {match ? (
              <span className="inline-flex items-center gap-1 rounded-[7px] bg-[var(--scry-success-bg)] px-2 py-0.5 font-bold text-[var(--scry-success-text-soft)]">
                <Heart className="h-3.5 w-3.5" />
                {match}
              </span>
            ) : null}
          </div>
          <TitleRatingsStrip
            ratings={{
              rating: item.rating,
              ratingSources: item.ratingSources ?? [],
              externalRatings: item.externalRatings ?? [],
            }}
            variant="hero"
          />
          {item.overview ? (
            <p className="m-0 line-clamp-4 max-w-[620px] text-[13.5px] leading-6 text-[#b7c0dd]">
              {item.overview}
            </p>
          ) : null}
          {genres.length > 0 ? (
            <div className="mt-4 flex flex-wrap gap-2">
              {genres.map((genre) => (
                <span
                  key={genre}
                  className="rounded-[8px] border border-white/10 bg-white/10 px-3 py-1.5 text-xs text-[#cfd7ee]"
                >
                  {genre}
                </span>
              ))}
            </div>
          ) : null}
        </div>
      </div>
      <div className="absolute inset-x-8 bottom-8 z-10 flex flex-col items-start gap-3 max-sm:static max-sm:mt-6 max-sm:items-stretch">
        <div className="flex justify-start">
          <DiscoveryActionButton
            item={item}
            canManageTitle={canManageTitle}
            canRequestMedia={canRequestMedia}
            onAction={onAction}
          />
        </div>
        {hasExternalLinks ? (
          <div className="flex flex-wrap items-center justify-start gap-2">
            <ImdbExternalLink
              imdbId={externalIds.bySource("imdb")}
              size="compact"
            />
            <TmdbExternalLink
              tmdbId={externalIds.bySourceKind("tmdb", tmdbMediaType)}
              mediaType={tmdbMediaType}
              size="compact"
            />
            {facet === "MOVIE" ? (
              <TvdbMovieExternalLink
                tvdbId={externalIds.bySourceKind("tvdb", tvdbKind)}
                size="compact"
              />
            ) : (
              <TvdbSeriesExternalLink
                tvdbId={externalIds.bySourceKind("tvdb", tvdbKind)}
                size="compact"
              />
            )}
            <MalExternalLink
              malId={externalIds.bySource("mal")}
              size="compact"
            />
            <AnilistExternalLink
              anilistId={externalIds.bySource("anilist")}
              size="compact"
            />
            <AnidbExternalLink
              anidbId={externalIds.bySource("anidb")}
              size="compact"
            />
          </div>
        ) : null}
      </div>
    </section>
  );
}

function DiscoveryFilterMultiSelect({
  options,
  selectedValues,
  placeholder,
  ariaLabel,
  onSelectedValuesChange,
}: {
  options: string[];
  selectedValues: string[];
  placeholder: string;
  ariaLabel: string;
  onSelectedValuesChange: (values: string[]) => void;
}) {
  const triggerLabel =
    selectedValues.length > 0
      ? selectedValues.length === 1
        ? selectedValues[0]
        : `${selectedValues.length} selected`
      : placeholder;

  return (
    <MultiSelectDropdown
      options={options.map((option) => ({ value: option, label: option }))}
      selectedValues={selectedValues}
      onSelectedValuesChange={onSelectedValuesChange}
      triggerLabel={triggerLabel}
      placeholder={placeholder}
      ariaLabel={ariaLabel}
      size="compact"
      chrome="toolbar"
    />
  );
}

function DiscoveryFilterChips({
  values,
  onRemove,
}: {
  values: string[];
  onRemove: (value: string) => void;
}) {
  if (values.length === 0) {
    return null;
  }

  return (
    <div className="mt-3 flex flex-wrap gap-2">
      {values.map((value) => (
        <button
          key={value}
          type="button"
          onClick={() => onRemove(value)}
          className="inline-flex max-w-full items-center gap-2 rounded-[8px] border border-[rgba(var(--scry-accent-rgb),0.34)] bg-[rgba(var(--scry-accent-rgb),0.15)] px-3 py-1.5 text-xs font-semibold text-[var(--scry-accent-text)] transition hover:border-[rgba(var(--scry-accent-rgb),0.48)] hover:bg-[rgba(var(--scry-accent-rgb),0.22)]"
        >
          <span className="truncate">{value}</span>
          <X className="h-3.5 w-3.5 opacity-75" aria-hidden="true" />
        </button>
      ))}
    </div>
  );
}

function DiscoveryFilters({
  variant = "desktop",
  items,
  availableContentTypes,
  selectedContentTypes,
  selectedGenres,
  selectedTags,
  selectedRelationTypes,
  selectedStudioSlugs,
  minimumYear,
  maximumYear,
  minimumRating,
  hiddenItemCount,
  onToggleContentType,
  onGenresChange,
  onTagsChange,
  onToggleRelationType,
  onToggleStudioSlug,
  onMinimumYearChange,
  onMaximumYearChange,
  onMinimumRatingChange,
  onClear,
  onShowHidden,
  onRequestClose,
}: {
  variant?: "desktop" | "mobile";
  items: DiscoveryItem[];
  availableContentTypes: DiscoveryContentType[];
  selectedContentTypes: DiscoveryContentType[];
  selectedGenres: string[];
  selectedTags: string[];
  selectedRelationTypes: string[];
  selectedStudioSlugs: string[];
  minimumYear: number;
  maximumYear: number;
  minimumRating: number;
  hiddenItemCount: number;
  onToggleContentType: (contentType: DiscoveryContentType) => void;
  onGenresChange: (genres: string[]) => void;
  onTagsChange: (tags: string[]) => void;
  onToggleRelationType: (relationType: string) => void;
  onToggleStudioSlug: (studioSlug: string) => void;
  onMinimumYearChange: (year: number) => void;
  onMaximumYearChange: (year: number) => void;
  onMinimumRatingChange: (rating: number) => void;
  onClear: () => void;
  onShowHidden: () => void;
  onRequestClose?: () => void;
}) {
  const t = useTranslate();
  const contentTypeCountItems = React.useMemo(
    () =>
      items.filter((item) =>
        itemMatchesDiscoveryFacetFilters(item, {
          genres: selectedGenres,
          tags: selectedTags,
          relationTypes: selectedRelationTypes,
          studioSlugs: selectedStudioSlugs,
          minimumYear,
          maximumYear,
          minimumRating,
        }),
      ),
    [
      items,
      maximumYear,
      minimumRating,
      minimumYear,
      selectedGenres,
      selectedRelationTypes,
      selectedStudioSlugs,
      selectedTags,
    ],
  );
  const relationTypeOptions = React.useMemo(
    () => presentRelationTypes(items),
    [items],
  );
  const studioSlugOptions = React.useMemo(
    () => presentStudioSlugs(items, selectedStudioSlugs),
    [items, selectedStudioSlugs],
  );
  const contentTypes: Array<{
    key: DiscoveryContentType;
    label: string;
    count: number;
  }> = availableContentTypes.map((key) => ({
    key,
    label:
      key === "MOVIE"
        ? t("discovery.type.movies")
        : key === "SERIES"
          ? t("discovery.type.series")
          : t("discovery.type.anime"),
    count: contentTypeCount(contentTypeCountItems, key),
  }));
  const genres = canonicalDiscoveryFilterOptions(items, "genre");
  const tags = canonicalDiscoveryFilterOptions(items, "theme");
  const minimumYearBound = DEFAULT_MINIMUM_YEAR;
  const maximumYearBound = defaultMaximumDiscoveryYear();
  const yearSpan = Math.max(1, maximumYearBound - minimumYearBound);
  const minimumYearPercent =
    ((minimumYear - minimumYearBound) / yearSpan) * 100;
  const maximumYearPercent =
    ((maximumYear - minimumYearBound) / yearSpan) * 100;
  const ratingPercent = (Math.min(Math.max(minimumRating, 0), 10) / 10) * 100;

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
      <div className="mb-5">
        <DiscoveryFilterMultiSelect
          options={genres}
          selectedValues={selectedGenres}
          placeholder={t("discovery.selectGenres")}
          ariaLabel={t("discovery.genres")}
          onSelectedValuesChange={onGenresChange}
        />
        <DiscoveryFilterChips
          values={selectedGenres}
          onRemove={(genre) =>
            onGenresChange(
              selectedGenres.filter((selectedGenre) => selectedGenre !== genre),
            )
          }
        />
      </div>
      <div className="mb-2.5 text-xs font-bold uppercase tracking-[0.05em] text-[var(--scry-muted2)]">
        {t("discovery.tags")}
      </div>
      <div className="mb-5">
        <DiscoveryFilterMultiSelect
          options={tags}
          selectedValues={selectedTags}
          placeholder={t("discovery.selectTags")}
          ariaLabel={t("discovery.tags")}
          onSelectedValuesChange={onTagsChange}
        />
        <DiscoveryFilterChips
          values={selectedTags}
          onRemove={(tag) =>
            onTagsChange(
              selectedTags.filter((selectedTag) => selectedTag !== tag),
            )
          }
        />
      </div>
      {relationTypeOptions.length > 0 ? (
        <>
          <div className="mb-2.5 text-xs font-bold uppercase tracking-[0.05em] text-[var(--scry-muted2)]">
            {t("discovery.relationship")}
          </div>
          <div className="mb-5 flex flex-wrap gap-2">
            {relationTypeOptions.map((relationType) => {
              const active = selectedRelationTypes.includes(relationType);
              return (
                <button
                  key={relationType}
                  type="button"
                  aria-pressed={active}
                  onClick={() => onToggleRelationType(relationType)}
                  className={cn(
                    "inline-flex items-center rounded-[8px] border px-3 py-1.5 text-xs font-semibold transition",
                    active
                      ? "border-[rgba(var(--scry-accent-rgb),0.48)] bg-[rgba(var(--scry-accent-rgb),0.22)] text-[var(--scry-accent-text)]"
                      : "border-[var(--scry-border2)] bg-[var(--scry-bg)] text-[var(--scry-text2)] hover:border-[rgba(var(--scry-accent-rgb),0.34)]",
                  )}
                >
                  {relationTypeLabel(relationType, t) ?? relationType}
                </button>
              );
            })}
          </div>
        </>
      ) : null}
      {studioSlugOptions.length > 0 ? (
        <>
          <div className="mb-2.5 text-xs font-bold uppercase tracking-[0.05em] text-[var(--scry-muted2)]">
            {t("discovery.studio")}
          </div>
          <div className="mb-5 flex flex-wrap gap-2">
            {studioSlugOptions.map((studioSlug) => {
              const active = selectedStudioSlugs.includes(studioSlug);
              return (
                <button
                  key={studioSlug}
                  type="button"
                  aria-pressed={active}
                  onClick={() => onToggleStudioSlug(studioSlug)}
                  className={cn(
                    "inline-flex max-w-full items-center rounded-[8px] border px-3 py-1.5 text-xs font-semibold transition",
                    active
                      ? "border-[rgba(var(--scry-accent-rgb),0.48)] bg-[rgba(var(--scry-accent-rgb),0.22)] text-[var(--scry-accent-text)]"
                      : "border-[var(--scry-border2)] bg-[var(--scry-bg)] text-[var(--scry-text2)] hover:border-[rgba(var(--scry-accent-rgb),0.34)]",
                  )}
                >
                  <span className="truncate">{studioSlugLabel(studioSlug)}</span>
                </button>
              );
            })}
          </div>
        </>
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
            FILTER_RANGE_THUMB_POINTER_CLASS_NAME,
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
            FILTER_RANGE_THUMB_POINTER_CLASS_NAME,
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
      <div className="relative h-5">
        <div className="absolute left-0 right-0 top-1/2 h-1.5 -translate-y-1/2 rounded-full bg-[#16203a]" />
        <div
          className="absolute left-0 top-1/2 h-1.5 -translate-y-1/2 rounded-full bg-gradient-to-r from-[var(--scry-accent)] to-[var(--scry-accent-ring)]"
          style={{ width: `${ratingPercent}%` }}
        />
        <input
          type="range"
          min={0}
          max={10}
          step={0.5}
          value={minimumRating}
          onChange={(event) => onMinimumRatingChange(Number(event.target.value))}
          className={cn(
            "absolute left-0 right-0 top-1/2 -translate-y-1/2",
            FILTER_RANGE_CLASS_NAME,
          )}
        />
      </div>
      {hiddenItemCount > 0 ? (
        <div className="mt-6 border-t border-[var(--scry-border3)] pt-4">
          <div className="mb-2.5 flex items-center justify-between gap-2">
            <span className="text-xs font-bold uppercase tracking-[0.05em] text-[var(--scry-muted2)]">
              {t("discovery.hiddenTitles")}
            </span>
            <span className="text-[11px] text-[var(--scry-faint)]">
              {hiddenItemCount}
            </span>
          </div>
          <button
            type="button"
            onClick={onShowHidden}
            className="inline-flex w-full items-center justify-center gap-2 rounded-[9px] border border-[var(--scry-border2)] bg-[var(--scry-bg)] px-3 py-2 text-[12.5px] font-semibold text-[var(--scry-text2)] transition hover:border-[rgba(var(--scry-accent-rgb),0.34)]"
          >
            <Eye className="h-3.5 w-3.5 text-[var(--scry-accent-text)]" />
            {t("discovery.showHidden")}
          </button>
        </div>
      ) : null}
    </aside>
  );
}

// --- Local-only "not interested" (SI2: no server state, no telemetry) ---

const HIDDEN_ITEMS_STORAGE_KEY = "scryer.discovery.hiddenItems.v1";

function readHiddenItemKeys(): string[] {
  if (typeof window === "undefined") {
    return [];
  }
  try {
    const raw = window.localStorage.getItem(HIDDEN_ITEMS_STORAGE_KEY);
    if (!raw) {
      return [];
    }
    const parsed = JSON.parse(raw) as unknown;
    return Array.isArray(parsed)
      ? parsed.filter((value): value is string => typeof value === "string")
      : [];
  } catch {
    return [];
  }
}

function writeHiddenItemKeys(keys: string[]) {
  if (typeof window === "undefined") {
    return;
  }
  try {
    window.localStorage.setItem(
      HIDDEN_ITEMS_STORAGE_KEY,
      JSON.stringify(keys),
    );
  } catch {
    // Storage may be unavailable (private mode / quota) — hiding stays
    // in-memory for the session, which is acceptable for a local preference.
  }
}

function useHiddenDiscoveryItems() {
  const [hiddenKeys, setHiddenKeys] = React.useState<Set<string>>(
    () => new Set(),
  );
  // Hydrate from storage on mount only (avoids SSR mismatch).
  React.useEffect(() => {
    setHiddenKeys(new Set(readHiddenItemKeys()));
  }, []);
  const hideItem = React.useCallback((item: DiscoveryItem) => {
    setHiddenKeys((current) => {
      const key = itemStableKey(item);
      if (current.has(key)) {
        return current;
      }
      const next = new Set(current);
      next.add(key);
      writeHiddenItemKeys([...next]);
      return next;
    });
  }, []);
  const resetHidden = React.useCallback(() => {
    setHiddenKeys((current) => {
      if (current.size === 0) {
        return current;
      }
      writeHiddenItemKeys([]);
      return new Set();
    });
  }, []);
  return { hiddenKeys, hideItem, resetHidden };
}

function sectionsWithoutHiddenItems(
  sections: DiscoverySection[],
  hiddenKeys: Set<string>,
) {
  if (hiddenKeys.size === 0) {
    return sections;
  }
  return sections
    .map((section) => ({
      ...section,
      items: section.items.filter(
        (item) => !hiddenKeys.has(itemStableKey(item)),
      ),
    }))
    .filter((section) => section.items.length > 0);
}

function sectionsForDiscoveryFacets(
  sections: DiscoverySection[],
  allowedFacets: ReadonlySet<Facet>,
) {
  return sections
    .map((section) => {
      const items = section.items.filter((item) => {
        const facet = discoveryItemFacet(item);
        return facet !== null && allowedFacets.has(facet);
      });
      const removedCount = section.items.length - items.length;
      return {
        ...section,
        totalCount: Math.max(items.length, section.totalCount - removedCount),
        items,
      };
    })
    .filter((section) => section.items.length > 0);
}

// --- Freshness indicator (SW5) ---

// Locale-aware "3 hours ago"-style phrasing without per-locale strings for the
// relative part. Falls back to null when the timestamp is missing/unparseable.
function formatRelativeTime(
  value: string | null | undefined,
): string | null {
  if (!value) {
    return null;
  }
  const timestamp = new Date(value).getTime();
  if (Number.isNaN(timestamp)) {
    return null;
  }
  const deltaSeconds = Math.round((timestamp - Date.now()) / 1000);
  const absSeconds = Math.abs(deltaSeconds);
  const locale =
    typeof document !== "undefined"
      ? document.documentElement.lang || undefined
      : undefined;
  const formatter = new Intl.RelativeTimeFormat(locale, { numeric: "auto" });
  const divisions: Array<{ amount: number; unit: Intl.RelativeTimeFormatUnit }> =
    [
      { amount: 60, unit: "second" },
      { amount: 60, unit: "minute" },
      { amount: 24, unit: "hour" },
      { amount: 7, unit: "day" },
      { amount: 4.34524, unit: "week" },
      { amount: 12, unit: "month" },
      { amount: Number.POSITIVE_INFINITY, unit: "year" },
    ];
  let unitValue = absSeconds;
  let chosenUnit: Intl.RelativeTimeFormatUnit = "second";
  for (const division of divisions) {
    if (unitValue < division.amount) {
      chosenUnit = division.unit;
      break;
    }
    unitValue /= division.amount;
    chosenUnit = division.unit;
  }
  const signedValue = Math.round(unitValue) * (deltaSeconds < 0 ? -1 : 1);
  return formatter.format(signedValue, chosenUnit);
}

function mostRecentSyncTimestamp(
  status: DiscoverySyncStatus | null | undefined,
): string | null {
  if (!status) {
    return null;
  }
  const candidates = [
    status.state.lastPublicFeedCompletedAt,
    status.state.lastIncrementalReloadCompletedAt,
    status.state.lastContextSnapshotCompletedAt,
    status.state.updatedAt,
  ].filter((value): value is string => Boolean(value));
  let newest: string | null = null;
  let newestMs = -Infinity;
  for (const value of candidates) {
    const ms = new Date(value).getTime();
    if (!Number.isNaN(ms) && ms > newestMs) {
      newestMs = ms;
      newest = value;
    }
  }
  return newest;
}

function DiscoveryFreshnessChip({
  status,
}: {
  status: DiscoverySyncStatus | null | undefined;
}) {
  const t = useTranslate();
  const timestamp = mostRecentSyncTimestamp(status);
  const relative = formatRelativeTime(timestamp);
  if (!relative) {
    return null;
  }
  const pendingChanges = status?.pendingContextChangeCount ?? 0;
  const stale = pendingChanges > 0;
  return (
    <div className="inline-flex items-center gap-2">
      <span
        className="inline-flex items-center gap-1.5 rounded-[8px] border border-[var(--scry-border2)] bg-[var(--scry-bg)] px-2.5 py-1 text-[11.5px] font-medium text-[var(--scry-muted)]"
        title={timestamp ?? undefined}
      >
        <Clock className="h-3.5 w-3.5 text-[var(--scry-faint2)]" aria-hidden="true" />
        {t("discovery.updatedRelative", { relative })}
      </span>
      {stale ? (
        <span
          className="inline-flex items-center gap-1.5 rounded-[8px] border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] px-2.5 py-1 text-[11.5px] font-medium text-[var(--scry-warning-text)]"
          title={t("discovery.updatePendingHint")}
        >
          {t("discovery.updatePending")}
        </span>
      ) : null}
    </div>
  );
}

export function DiscoveryView({
  home,
  loading,
  error,
  manageableFacets,
  requestableFacets,
  onRefresh,
  onAction,
}: DiscoveryViewProps) {
  const t = useTranslate();
  const manageableFacetSet = React.useMemo(
    () => new Set(manageableFacets),
    [manageableFacets],
  );
  const requestableFacetSet = React.useMemo(
    () => new Set(requestableFacets),
    [requestableFacets],
  );
  const discoverableFacets = React.useMemo(
    () =>
      DISCOVERY_CONTENT_TYPES.filter(
        (facet) =>
          manageableFacetSet.has(facet) || requestableFacetSet.has(facet),
      ),
    [manageableFacetSet, requestableFacetSet],
  );
  const discoverableFacetSet = React.useMemo(
    () => new Set(discoverableFacets),
    [discoverableFacets],
  );
  const [selectedContentTypes, setSelectedContentTypes] = React.useState<
    DiscoveryContentType[]
  >(DEFAULT_DISCOVERY_CONTENT_TYPES);
  React.useEffect(() => {
    setSelectedContentTypes((current) => {
      const visibleSelection = current.filter((contentType) =>
        discoverableFacetSet.has(contentType),
      );
      const next =
        visibleSelection.length > 0
          ? visibleSelection
          : [...discoverableFacets];
      return current.length === next.length &&
        current.every((contentType, index) => contentType === next[index])
        ? current
        : next;
    });
  }, [discoverableFacetSet, discoverableFacets]);
  const [selectedGenres, setSelectedGenres] = React.useState<string[]>([]);
  const [selectedTags, setSelectedTags] = React.useState<string[]>([]);
  const [minimumYear, setMinimumYear] =
    React.useState(DEFAULT_MINIMUM_YEAR);
  const [maximumYear, setMaximumYear] = React.useState(
    defaultMaximumDiscoveryYear,
  );
  const [minimumRating, setMinimumRating] = React.useState(
    DEFAULT_MINIMUM_RATING,
  );
  const [selectedRelationTypes, setSelectedRelationTypes] = React.useState<
    string[]
  >([]);
  const [selectedStudioSlugs, setSelectedStudioSlugs] = React.useState<
    string[]
  >([]);
  const [filtersOpen, setFiltersOpen] = React.useState(false);
  const { hiddenKeys, hideItem, resetHidden } = useHiddenDiscoveryItems();
  const orderedSections = React.useMemo(
    () => orderedHomeSections(home),
    [home],
  );
  const capabilitySections = React.useMemo(
    () => sectionsForDiscoveryFacets(orderedSections, discoverableFacetSet),
    [discoverableFacetSet, orderedSections],
  );
  // Local-only "not interested": drop hidden items before any filtering so
  // filter option lists and counts reflect what the user actually sees.
  const rawSections = React.useMemo(
    () => sectionsWithoutHiddenItems(capabilitySections, hiddenKeys),
    [capabilitySections, hiddenKeys],
  );
  const rawItems = React.useMemo(
    () => rawSections.flatMap((section) => section.items),
    [rawSections],
  );
  const hiddenItemCount = React.useMemo(() => {
    if (hiddenKeys.size === 0) {
      return 0;
    }
    const visibleFeedKeys = new Set(
      capabilitySections
        .flatMap((section) => section.items)
        .map((item) => itemStableKey(item)),
    );
    let count = 0;
    for (const key of hiddenKeys) {
      if (visibleFeedKeys.has(key)) {
        count += 1;
      }
    }
    return count;
  }, [capabilitySections, hiddenKeys]);
  const yearBounds = React.useMemo(
    () => ({
      minimum: DEFAULT_MINIMUM_YEAR,
      maximum: defaultMaximumDiscoveryYear(),
    }),
    [],
  );
  const effectiveMaximumYear = Math.max(
    Math.min(Math.max(maximumYear, yearBounds.minimum), yearBounds.maximum),
    yearBounds.minimum,
  );
  const effectiveMinimumYear = Math.min(
    Math.max(minimumYear, yearBounds.minimum),
    effectiveMaximumYear,
  );
  const effectiveSelectedContentTypes = React.useMemo(
    () =>
      selectedContentTypes.filter((contentType) =>
        discoverableFacetSet.has(contentType),
      ),
    [discoverableFacetSet, selectedContentTypes],
  );
  const sections = React.useMemo(
    () =>
      filterDiscoverySections(rawSections, {
        contentTypes: effectiveSelectedContentTypes,
        genres: selectedGenres,
        tags: selectedTags,
        relationTypes: selectedRelationTypes,
        studioSlugs: selectedStudioSlugs,
        minimumYear: effectiveMinimumYear,
        maximumYear: effectiveMaximumYear,
        minimumRating,
      }),
    [
      effectiveMaximumYear,
      effectiveMinimumYear,
      effectiveSelectedContentTypes,
      minimumRating,
      rawSections,
      selectedGenres,
      selectedRelationTypes,
      selectedStudioSlugs,
      selectedTags,
    ],
  );
  const heroSections = React.useMemo(
    () => sections.filter((section) => !sectionIsCompleteCollection(section)),
    [sections],
  );
  const heroItem = React.useMemo(
    () => {
      const configuredHeroFacet = home?.heroItem
        ? discoveryItemFacet(home.heroItem)
        : null;
      return home?.heroItem &&
        configuredHeroFacet !== null &&
        discoverableFacetSet.has(configuredHeroFacet) &&
        discoveryItemHasUsefulTitle(home.heroItem)
        ? home.heroItem
        : firstHeroItem(heroSections);
    },
    [discoverableFacetSet, heroSections, home?.heroItem],
  );
  const heroFacet = heroItem ? discoveryItemFacet(heroItem) : null;
  const heroRailSection = React.useMemo(
    () => findHeroRailSection(heroSections),
    [heroSections],
  );
  const heroRailSectionWithoutHero = React.useMemo(
    () => sectionWithoutItem(heroRailSection, heroItem),
    [heroItem, heroRailSection],
  );
  const railSections = React.useMemo(
    () =>
      sections
        .filter((section) => section.sectionId !== heroRailSection?.sectionId)
        .map((section) => sectionWithoutItem(section, heroItem))
        .filter((section): section is DiscoverySection => Boolean(section)),
    [heroItem, heroRailSection, sections],
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
    upcomingRailSections.length > 0;
  const freshnessTimestamp = React.useMemo(
    () => mostRecentSyncTimestamp(home?.status),
    [home?.status],
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
  const toggleRelationType = React.useCallback((relationType: string) => {
    setSelectedRelationTypes((current) =>
      current.includes(relationType)
        ? current.filter((value) => value !== relationType)
        : [...current, relationType],
    );
  }, []);
  const toggleStudioSlug = React.useCallback((studioSlug: string) => {
    setSelectedStudioSlugs((current) =>
      current.includes(studioSlug)
        ? current.filter((value) => value !== studioSlug)
        : [...current, studioSlug],
    );
  }, []);
  const clearFilters = React.useCallback(() => {
    setSelectedContentTypes(discoverableFacets);
    setSelectedGenres([]);
    setSelectedTags([]);
    setSelectedRelationTypes([]);
    setSelectedStudioSlugs([]);
    setMinimumYear(Math.max(yearBounds.minimum, DEFAULT_MINIMUM_YEAR));
    setMaximumYear(yearBounds.maximum);
    setMinimumRating(DEFAULT_MINIMUM_RATING);
  }, [discoverableFacets, yearBounds.maximum, yearBounds.minimum]);
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
    items: rawItems,
    availableContentTypes: discoverableFacets,
    selectedContentTypes: effectiveSelectedContentTypes,
    selectedGenres,
    selectedTags,
    selectedRelationTypes,
    selectedStudioSlugs,
    minimumYear: effectiveMinimumYear,
    maximumYear: effectiveMaximumYear,
    minimumRating,
    hiddenItemCount,
    onToggleContentType: toggleContentType,
    onGenresChange: setSelectedGenres,
    onTagsChange: setSelectedTags,
    onToggleRelationType: toggleRelationType,
    onToggleStudioSlug: toggleStudioSlug,
    onMinimumYearChange: setMinimumYear,
    onMaximumYearChange: setMaximumYear,
    onMinimumRatingChange: setMinimumRating,
    onClear: clearFilters,
    onShowHidden: resetHidden,
  };

  if (loading && !home) {
    return (
      <div className="flex min-h-[360px] items-center justify-center">
        <Loader2 className="h-6 w-6 animate-spin text-[var(--scry-accent)]" />
      </div>
    );
  }

  return (
    <div
      id="discovery-view"
      data-ui="discovery-view"
      className="flex min-h-0 flex-1"
    >
      <main className="min-w-0 flex-1 overflow-y-auto px-7 py-6 pb-16 max-sm:px-4">
        <div
          className={cn(
            "mb-5 items-center justify-between gap-3",
            // Always present below xl (holds the mobile filters button); on xl
            // only when there is a freshness chip to show.
            "flex max-xl:flex",
            freshnessTimestamp ? "xl:flex" : "xl:hidden",
          )}
        >
          <DiscoveryFreshnessChip status={home?.status} />
          <button
            type="button"
            aria-label={t("discovery.openFilters")}
            onClick={() => setFiltersOpen(true)}
            className="inline-flex h-9 shrink-0 items-center gap-2 rounded-[9px] border border-[var(--scry-border2)] bg-[var(--scry-bg)] px-3 text-[12.5px] font-semibold text-[var(--scry-ink2)] max-xl:inline-flex xl:hidden"
          >
            <SlidersHorizontal className="h-4 w-4 text-[var(--scry-accent-text)]" />
            {t("discovery.filters")}
          </button>
        </div>

        {error ? (
          <div className="mb-5 flex items-center justify-between gap-4 rounded-[12px] border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] px-4 py-3 text-sm text-[var(--scry-danger-text)]">
            <span>{error}</span>
            <Button type="button" size="sm" variant="outline" onClick={onRefresh}>
              {t("label.retry")}
            </Button>
          </div>
        ) : null}

        {heroItem ? (
          <div className="mb-7 grid grid-cols-[minmax(0,1.15fr)_minmax(0,1fr)] items-stretch gap-5 max-lg:grid-cols-1 lg:h-[clamp(440px,46vh,520px)]">
            <DiscoveryHero
              item={heroItem}
              canManageTitle={
                heroFacet !== null && manageableFacetSet.has(heroFacet)
              }
              canRequestMedia={
                heroFacet !== null && requestableFacetSet.has(heroFacet)
              }
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
                fillHeight
                manageableFacets={manageableFacetSet}
                requestableFacets={requestableFacetSet}
                onAction={onAction}
                onDismissItem={hideItem}
              />
            ) : null}
          </div>
        ) : null}

        {primaryRailSections.length > 0 ? (
          primaryRailSections.map((section) => (
            <DiscoverySectionRail
              key={section.sectionId}
              section={section}
              manageableFacets={manageableFacetSet}
              requestableFacets={requestableFacetSet}
              onAction={onAction}
              onDismissItem={hideItem}
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

        {upcomingRailSections.map((section) => (
          <DiscoverySectionRail
            key={section.sectionId}
            section={section}
            variant="upcoming"
            manageableFacets={manageableFacetSet}
            requestableFacets={requestableFacetSet}
            onAction={onAction}
            onDismissItem={hideItem}
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
