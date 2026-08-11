import assert from "node:assert/strict";
import test from "node:test";

import { indexerDownloadClientMappingCatalogQuery } from "../graphql/queries.ts";
import en from "../i18n/locales/en.ts";
import type {
  IndexerDownloadClientMappingCatalog,
  IndexerDownloadClientMappingClient,
} from "../types/index.ts";
import {
  AUTOMATIC_DOWNLOAD_CLIENT_ID,
  getIndexerDownloadClientMappingViewModel,
  isManagementOnlyIndexer,
  updateIndexerDownloadClientMapping,
  updatePendingIndexerMappingIds,
} from "./indexer-download-client-mapping.ts";

const directIndexer = {
  id: "indexer-direct",
  isManaged: false,
  supportsManagedChildrenSync: false,
};

const clients: IndexerDownloadClientMappingClient[] = [
  {
    id: "usenet-a",
    name: "Usenet A",
    clientType: "sabnzbd",
    isEnabled: true,
    healthStatus: "healthy",
  },
  {
    id: "usenet-disabled",
    name: "Disabled Usenet",
    clientType: "nzbget",
    isEnabled: false,
    healthStatus: "disabled",
  },
  {
    id: "torrent-a",
    name: "Torrent A",
    clientType: "qbittorrent",
    isEnabled: true,
    healthStatus: "healthy",
  },
];

function catalog(
  mapping: Partial<IndexerDownloadClientMappingCatalog["indexers"][number]> = {},
): IndexerDownloadClientMappingCatalog {
  return {
    clients,
    indexers: [
      {
        id: directIndexer.id,
        name: "Direct Indexer",
        protocolFamilies: ["usenet"],
        supportsMapping: true,
        compatibleClientIds: ["usenet-a", "usenet-disabled"],
        downloadClientId: null,
        ...mapping,
      },
    ],
  };
}

test("mapping choices include only compatible clients and Automatic first", () => {
  const model = getIndexerDownloadClientMappingViewModel(directIndexer, catalog());

  assert.equal(model.selectedId, AUTOMATIC_DOWNLOAD_CLIENT_ID);
  assert.deepEqual(model.compatibleClients.map((option) => option.id), [
    "usenet-a",
    "usenet-disabled",
  ]);
  assert.equal(model.options.some((option) => option.id === "torrent-a"), false);
});

test("compatible disabled clients remain selectable and are marked disabled", () => {
  const model = getIndexerDownloadClientMappingViewModel(
    directIndexer,
    catalog({ downloadClientId: "usenet-disabled" }),
  );

  assert.equal(model.isInvalid, false);
  assert.equal(model.isDisabled, true);
  assert.equal(model.options.find((option) => option.id === "usenet-disabled")?.enabled, false);
});

test("missing, incompatible, and unavailable current values stay visible as invalid", () => {
  const missing = getIndexerDownloadClientMappingViewModel(
    directIndexer,
    catalog({ downloadClientId: "removed-client" }),
  );
  assert.equal(missing.invalidReason, "missing");
  assert.equal(missing.options[0]?.id, "removed-client");

  const incompatible = getIndexerDownloadClientMappingViewModel(
    directIndexer,
    catalog({ downloadClientId: "torrent-a" }),
  );
  assert.equal(incompatible.invalidReason, "incompatible");
  assert.equal(incompatible.options[0]?.id, "torrent-a");

  const unavailableCatalog = catalog({ downloadClientId: "usenet-a" });
  unavailableCatalog.clients = clients.map((client) =>
    client.id === "usenet-a" ? { ...client, healthStatus: "offline" } : client,
  );
  const unavailable = getIndexerDownloadClientMappingViewModel(
    directIndexer,
    unavailableCatalog,
  );
  assert.equal(unavailable.invalidReason, "unavailable");
  assert.equal(unavailable.options[0]?.id, "usenet-a");
});

test("managed children get mappings while management-only parents do not", () => {
  const parent = {
    id: "prowlarr-parent",
    isManaged: false,
    supportsManagedChildrenSync: true,
  };
  const child = {
    id: "prowlarr-child",
    isManaged: true,
    supportsManagedChildrenSync: false,
  };

  assert.equal(isManagementOnlyIndexer(parent), true);
  assert.equal(
    getIndexerDownloadClientMappingViewModel(parent, catalog()).isNotApplicable,
    true,
  );
  assert.equal(
    getIndexerDownloadClientMappingViewModel(child, {
      ...catalog(),
      indexers: [{ ...catalog().indexers[0]!, id: child.id }],
    }).isNotApplicable,
    false,
  );
});

test("pending state is row-scoped and mapping updates can roll back", () => {
  const pending = updatePendingIndexerMappingIds(new Set(["other-row"]), directIndexer.id, true);
  assert.equal(pending.has(directIndexer.id), true);
  assert.equal(pending.has("other-row"), true);

  const optimistic = updateIndexerDownloadClientMapping(catalog(), directIndexer.id, "usenet-a");
  const rolledBack = updateIndexerDownloadClientMapping(optimistic, directIndexer.id, null);
  assert.equal(rolledBack.indexers[0]?.downloadClientId, null);
  assert.equal(updatePendingIndexerMappingIds(pending, directIndexer.id, false).has("other-row"), true);
});

test("mapping catalog and deletion copy carry the integration contract", () => {
  assert.equal(indexerDownloadClientMappingCatalogQuery.includes("indexerDownloadClientMappingCatalog"), true);
  assert.equal(indexerDownloadClientMappingCatalogQuery.includes("compatibleClientIds"), true);
  assert.equal(en["settings.downloadClientDeleteConfirmDescription"]?.includes("Automatic"), true);
  assert.equal(en["status.downloadClientDeletedWithMappings"]?.includes("{{count}}"), true);
});
