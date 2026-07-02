import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import { canonicalDiscoveryFacetLabels } from "@/lib/discovery-facets";
import type { CatalogDiscoveryItem, ExternalId, Facet } from "@/lib/types";
import { discoveryItemDisplayTitle } from "@/lib/utils/discovery-display";

export function normalizedDiscoveryItemFacet(
  value: string | null | undefined,
): Facet | null {
  switch (value?.trim().toLowerCase()) {
    case "anime":
      return "anime";
    case "series":
      return "series";
    case "movie":
      return "movie";
    default:
      return null;
  }
}

export function discoveryItemFacet(item: CatalogDiscoveryItem): Facet | null {
  const contentType = item.contentType?.trim();
  return contentType
    ? normalizedDiscoveryItemFacet(contentType)
    : normalizedDiscoveryItemFacet(item.targetKind);
}

type DiscoveryExternalIdSignals = {
  targetKey: string;
  sourceTags?: string[] | null;
};

const EXTERNAL_ID_SOURCE_ALIASES: Record<string, string> = {
  anidb: "anidb",
  anidbnet: "anidb",
  anilist: "anilist",
  anilistco: "anilist",
  imdb: "imdb",
  imdbcom: "imdb",
  mal: "mal",
  myanimelist: "mal",
  myanimelistnet: "mal",
  themoviedb: "tmdb",
  themoviedborg: "tmdb",
  tmdb: "tmdb",
  trakt: "trakt",
  traktv: "trakt",
  thetvdb: "tvdb",
  thetvdbcom: "tvdb",
  tvdb: "tvdb",
};

function normalizedExternalIdSource(value: string | null | undefined) {
  const normalized = value?.trim().toLowerCase().replace(/[\s_.-]+/g, "");
  return normalized ? (EXTERNAL_ID_SOURCE_ALIASES[normalized] ?? null) : null;
}

function externalIdFromUrl(value: string): ExternalId | null {
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    return null;
  }

  const host = url.hostname.toLowerCase().replace(/^www\./, "");
  const path = url.pathname;
  if (host === "imdb.com") {
    const match = path.match(/\/title\/(tt\d+)/i);
    return match ? { source: "imdb", value: match[1] } : null;
  }
  if (host === "themoviedb.org") {
    const match = path.match(/\/(?:movie|tv)\/(\d+)/i);
    return match ? { source: "tmdb", value: match[1] } : null;
  }
  if (host === "thetvdb.com") {
    const match = path.match(
      /\/(?:dereferrer\/(?:movie|series)|movies?|series)\/(\d+)/i,
    );
    return match ? { source: "tvdb", value: match[1] } : null;
  }
  if (host === "myanimelist.net") {
    const match = path.match(/\/anime\/(\d+)/i);
    return match ? { source: "mal", value: match[1] } : null;
  }
  if (host === "anilist.co") {
    const match = path.match(/\/anime\/(\d+)/i);
    return match ? { source: "anilist", value: match[1] } : null;
  }
  if (host === "anidb.net") {
    const match = path.match(/\/anime\/(\d+)/i);
    return match ? { source: "anidb", value: match[1] } : null;
  }
  return null;
}

function externalIdFromDiscoveryKey(value: string): ExternalId | null {
  const urlExternalId = externalIdFromUrl(value);
  if (urlExternalId) {
    return urlExternalId;
  }

  const parts = value
    .split(":")
    .map((part) => part.trim())
    .filter(Boolean);
  const source = normalizedExternalIdSource(parts[0]);
  if (!source) {
    return null;
  }
  const id =
    source === "imdb"
      ? (parts.find((part) => /^tt\d+$/i.test(part)) ?? parts.at(-1))
      : parts.at(-1);
  return id ? { source, value: id } : null;
}

export function externalIdsFromDiscoverySignals(
  item: DiscoveryExternalIdSignals,
): ExternalId[] {
  const ids = new Map<string, string>();
  for (const candidate of [item.targetKey, ...(item.sourceTags ?? [])]) {
    const externalId = externalIdFromDiscoveryKey(candidate);
    if (externalId && !ids.has(externalId.source)) {
      ids.set(externalId.source, externalId.value);
    }
  }
  return Array.from(ids, ([source, value]) => ({ source, value }));
}

export function externalIdsForDiscoveryItem(
  item: CatalogDiscoveryItem,
): ExternalId[] {
  return externalIdsFromDiscoverySignals(item);
}

export function metadataResultForDiscoveryItem(
  item: CatalogDiscoveryItem,
): MetadataTvdbSearchItem {
  const externalIds = externalIdsForDiscoveryItem(item);
  return {
    tvdbId:
      externalIds.find((externalId) => externalId.source === "tvdb")?.value ??
      "",
    name: discoveryItemDisplayTitle(item),
    imdbId:
      externalIds.find((externalId) => externalId.source === "imdb")?.value ??
      null,
    externalIds,
    slug: null,
    type: item.contentType ?? item.targetKind,
    year: item.year,
    status: item.statusTags[0] ?? null,
    overview: item.overview ?? null,
    popularity: item.rankScore,
    posterUrl: item.posterUrl,
    backgroundUrl: item.backgroundUrl ?? null,
    language: null,
    runtimeMinutes: null,
    sortTitle: item.sortTitle,
    genres:
      item.facetTerms && item.facetTerms.length > 0
        ? canonicalDiscoveryFacetLabels(
            { facetTerms: item.facetTerms },
            "genre",
          )
        : item.genres,
    rating: item.rating ?? null,
    ratingSource: item.sources?.[0] ?? item.bestSource ?? null,
    externalRatings: item.externalRatings ?? [],
  };
}
