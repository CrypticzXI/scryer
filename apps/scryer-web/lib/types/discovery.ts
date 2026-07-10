import type { TitleExternalRating } from "@/lib/utils/title-ratings";
import type { CanonicalMediaTag } from "./canonical-tags";

export type DiscoveryExternalId = {
  source: string;
  kind: string;
  id: string;
  key: string;
};

export type DiscoveryHomeInput = {
  includePublic?: boolean | null;
  includePersonalized?: boolean | null;
  includeUnresolved?: boolean | null;
  limitPerSection?: number | null;
};

export type DiscoverySyncState = {
  lastSuccessGenerationId: string | null;
  lastPublicFeedGenerationId: string | null;
  lastContextSnapshotCompletedAt: string | null;
  lastIncrementalReloadCompletedAt: string | null;
  lastPublicFeedCompletedAt: string | null;
  nextContextSnapshotEligibleAt: string | null;
  nextIncrementalReloadEligibleAt: string | null;
  nextPublicFeedEligibleAt: string | null;
  updatedAt: string;
};

export type DiscoverySyncStatus = {
  pendingContextChangeCount: number;
  state: DiscoverySyncState;
};

export type DiscoveryFacet = {
  name: string;
  value: string;
  smgCount: number | null;
  localCount: number | null;
};

export type DiscoveryItem = {
  id: string;
  targetKey: string;
  targetKind: string;
  resolved: boolean;
  resolvedTitleId: string | null;
  displayTitle: string;
  originalTitle: string | null;
  sortTitle: string | null;
  year: number | null;
  posterUrl: string | null;
  backgroundUrl: string | null;
  overview: string | null;
  contentType: string | null;
  canonicalTags?: CanonicalMediaTag[];
  rating: number | null;
  ratingSources?: string[];
  externalRatings?: TitleExternalRating[];
  externalIds?: DiscoveryExternalId[];
  statusTags: string[];
  sourceTags: string[];
  sources: string[];
  bestSource: string | null;
  relationTypes: string[];
  relationSubtypes: string[];
  sourceCount: number | null;
  edgeCount: number | null;
  relationCount: number | null;
  sourceSubjectCount: number | null;
  rankScore: number | null;
  matchedSubjectTitles: string[];
  matchedSubjectCount: number;
  tmdbCollectionId: string | null;
  tmdbCollectionName: string | null;
  ownedInInput: boolean;
  facetTerms: string[];
  contextTerms: string[];
  studioSlug: string | null;
  personIds: number[];
};

export type DiscoverySection = {
  sectionId: string;
  sectionType: string;
  title: string;
  surface: string;
  totalCount: number;
  items: DiscoveryItem[];
};

export type DiscoveryHomePayload = {
  status: DiscoverySyncStatus;
  heroItem: DiscoveryItem | null;
  publicSections: DiscoverySection[];
  personalizedSections: DiscoverySection[];
  completeCollection: DiscoverySection | null;
  facets: DiscoveryFacet[];
  canViewPersonalized: boolean;
};

export type DiscoveryItemsInput = {
  query?: string | null;
  targetKinds?: string[] | null;
  sources?: string[] | null;
  relationTypes?: string[] | null;
  relationSubtypes?: string[] | null;
  genres?: string[] | null;
  statusTags?: string[] | null;
  facetTerms?: string[] | null;
  includeOwned?: boolean | null;
  includeUnresolved?: boolean | null;
  includePublic?: boolean | null;
  limit?: number | null;
  offset?: number | null;
};

export type DiscoveryItemsPayload = {
  items: DiscoveryItem[];
  totalCount: number;
  canViewPersonalized: boolean;
};

export type CatalogDiscoveryInput = {
  facet: "MOVIE" | "SERIES" | "ANIME";
  libraryIds?: string[] | null;
  includeUnresolved?: boolean | null;
  limitPerGroup?: number | null;
  maxGroups?: number | null;
};

export type CatalogDiscoveryItem = Pick<
  DiscoveryItem,
  | "id"
  | "targetKey"
  | "targetKind"
  | "resolved"
  | "resolvedTitleId"
  | "displayTitle"
  | "originalTitle"
  | "sortTitle"
  | "year"
  | "posterUrl"
  | "contentType"
  | "statusTags"
  | "sourceTags"
  | "rankScore"
  | "ownedInInput"
> &
  Partial<
    Pick<
      DiscoveryItem,
      | "backgroundUrl"
      | "bestSource"
      | "canonicalTags"
      | "facetTerms"
      | "externalRatings"
      | "externalIds"
      | "overview"
      | "rating"
      | "sources"
    >
  >;

export type CatalogDiscoveryGroupKind =
  | "PUBLIC_TOP"
  | "PUBLIC_SECTION"
  | "GENRE_AFFINITY"
  | "THEME_AFFINITY"
  | "ACCLAIMED"
  | "COMPLETE_COLLECTION"
  | "FALLBACK";

export type CatalogDiscoverySurface = "PUBLIC" | "PERSONALIZED";

export type CatalogDiscoveryGroup = {
  id: string;
  kind: CatalogDiscoveryGroupKind;
  surface: CatalogDiscoverySurface;
  labelValue: string | null;
  totalCount: number;
  items: CatalogDiscoveryItem[];
};

export type CatalogDiscoveryPayload = {
  canViewPersonalized: boolean;
  groups: CatalogDiscoveryGroup[];
};
