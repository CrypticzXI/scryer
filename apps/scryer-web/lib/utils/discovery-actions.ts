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

export function externalIdsForDiscoveryItem(
  item: CatalogDiscoveryItem,
): ExternalId[] {
  const parts = item.targetKey.split(":").map((part) => part.trim());
  const source = parts[0]?.toLowerCase() ?? "";
  const value =
    parts.length >= 3
      ? parts.slice(2).join(":")
      : parts.length === 2
        ? parts[1]
        : "";
  return source && value ? [{ source, value }] : [];
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
