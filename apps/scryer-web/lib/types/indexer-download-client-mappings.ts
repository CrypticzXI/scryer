export type IndexerDownloadClientMappingClient = {
  id: string;
  name: string;
  clientType: string;
  isEnabled: boolean;
  healthStatus: string;
};

export type IndexerDownloadClientMappingIndexer = {
  id: string;
  name: string;
  downloadClientId: string | null;
  protocolFamilies: string[];
  supportsMapping: boolean;
  compatibleClientIds: string[];
};

export type IndexerDownloadClientMappingCatalog = {
  clients: IndexerDownloadClientMappingClient[];
  indexers: IndexerDownloadClientMappingIndexer[];
};
