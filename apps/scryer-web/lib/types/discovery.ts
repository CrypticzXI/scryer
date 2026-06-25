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
  genres: string[];
  rating: number | null;
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
  publicSections: DiscoverySection[];
  personalizedSections: DiscoverySection[];
  completeCollection: DiscoverySection | null;
  facets: DiscoveryFacet[];
  canViewPersonalized: boolean;
};
