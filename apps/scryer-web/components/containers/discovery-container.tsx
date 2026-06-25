import { memo, useCallback, useEffect, useState } from "react";
import { useClient } from "urql";
import {
  AddToCatalogDialog,
  EMPTY_SEARCH_RESULT,
} from "@/components/root/add-to-catalog-dialog";
import { RequestMediaDialog } from "@/components/root/request-media-dialog";
import { DiscoveryView } from "@/components/views/discovery-view";
import { discoveryHomeQuery } from "@/lib/graphql/queries";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useSearchContext } from "@/lib/context/search-context";
import { useTranslate } from "@/lib/context/translate-context";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import type {
  DiscoveryHomeInput,
  DiscoveryHomePayload,
  DiscoveryItem,
  ExternalId,
  Facet,
} from "@/lib/types";

type DiscoveryContainerProps = {
  canManageTitle: boolean;
  canRequestMedia: boolean;
};

const DISCOVERY_HOME_INPUT: DiscoveryHomeInput = {
  includePublic: true,
  includePersonalized: true,
  includeUnresolved: true,
  limitPerSection: 18,
};

function facetForDiscoveryItem(item: DiscoveryItem): Facet {
  const raw = `${item.targetKind} ${item.contentType ?? ""} ${item.facetTerms.join(" ")}`.toLowerCase();
  if (raw.includes("anime")) {
    return "anime";
  }
  if (raw.includes("series") || raw.includes("show")) {
    return "series";
  }
  return "movie";
}

function externalIdsForDiscoveryItem(item: DiscoveryItem): ExternalId[] {
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

function metadataResultForDiscoveryItem(
  item: DiscoveryItem,
): MetadataTvdbSearchItem {
  const externalIds = externalIdsForDiscoveryItem(item);
  return {
    tvdbId:
      externalIds.find((externalId) => externalId.source === "tvdb")?.value ?? "",
    name: item.displayTitle,
    imdbId:
      externalIds.find((externalId) => externalId.source === "imdb")?.value ??
      null,
    externalIds,
    slug: null,
    type: item.contentType ?? item.targetKind,
    year: item.year,
    status: item.statusTags[0] ?? null,
    overview: item.overview,
    popularity: item.rankScore,
    posterUrl: item.posterUrl,
    language: null,
    runtimeMinutes: null,
    sortTitle: item.sortTitle,
  };
}

export const DiscoveryContainer = memo(function DiscoveryContainer({
  canManageTitle,
  canRequestMedia,
}: DiscoveryContainerProps) {
  const t = useTranslate();
  const client = useClient();
  const setGlobalStatus = useGlobalStatus();
  const {
    addMetadataSearchResultToCatalog,
    catalogConfigLoading,
    catalogQualityProfileOptions,
    ensureCatalogConfigReady,
    librariesByFacet,
    requestMetadataSearchResult,
    requestableLibrariesByFacet,
    resolveDefaultQualityProfileIdForFacet,
    rootFoldersByFacet,
  } = useSearchContext();
  const [home, setHome] = useState<DiscoveryHomePayload | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selectedItem, setSelectedItem] = useState<DiscoveryItem | null>(null);
  const [addDialogOpen, setAddDialogOpen] = useState(false);
  const [requestDialogOpen, setRequestDialogOpen] = useState(false);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const { data, error: queryError } = await client
        .query(
          discoveryHomeQuery,
          { input: DISCOVERY_HOME_INPUT },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (queryError) {
        throw queryError;
      }
      setHome((data?.discoveryHome ?? null) as DiscoveryHomePayload | null);
    } catch (caught) {
      const message =
        caught instanceof Error ? caught.message : t("discovery.failedToLoad");
      setError(message);
      setGlobalStatus(message);
    } finally {
      setLoading(false);
    }
  }, [client, setGlobalStatus, t]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const selectedFacet = selectedItem
    ? facetForDiscoveryItem(selectedItem)
    : "movie";
  const selectedResult = selectedItem
    ? metadataResultForDiscoveryItem(selectedItem)
    : EMPTY_SEARCH_RESULT;

  const handleAction = useCallback(
    async (item: DiscoveryItem) => {
      if (item.ownedInInput) {
        return;
      }
      if (!canManageTitle && !canRequestMedia) {
        setGlobalStatus(t("status.permissionDenied"));
        return;
      }

      const facet = facetForDiscoveryItem(item);
      setSelectedItem(item);
      try {
        await ensureCatalogConfigReady(facet);
        if (canManageTitle) {
          setAddDialogOpen(true);
        } else {
          setRequestDialogOpen(true);
        }
      } catch (caught) {
        setGlobalStatus(
          caught instanceof Error ? caught.message : t("status.apiError"),
        );
      }
    },
    [
      canManageTitle,
      canRequestMedia,
      ensureCatalogConfigReady,
      setGlobalStatus,
      t,
    ],
  );

  const handleAddDialogOpenChange = useCallback((open: boolean) => {
    setAddDialogOpen(open);
    if (!open) {
      setSelectedItem(null);
    }
  }, []);

  const handleRequestDialogOpenChange = useCallback((open: boolean) => {
    setRequestDialogOpen(open);
    if (!open) {
      setSelectedItem(null);
    }
  }, []);

  return (
    <>
      <DiscoveryView
        home={home}
        loading={loading}
        error={error}
        canManageTitle={canManageTitle}
        canRequestMedia={canRequestMedia}
        onRefresh={refresh}
        onAction={handleAction}
      />
      {canManageTitle ? (
        <AddToCatalogDialog
          open={addDialogOpen}
          onOpenChange={handleAddDialogOpenChange}
          result={selectedResult}
          facet={selectedFacet}
          catalogQualityProfileOptions={catalogQualityProfileOptions}
          catalogConfigLoading={catalogConfigLoading}
          defaultQualityProfileId={resolveDefaultQualityProfileIdForFacet(
            selectedFacet,
          )}
          manageableLibraries={librariesByFacet[selectedFacet] ?? []}
          rootFolderOptions={rootFoldersByFacet[selectedFacet] ?? []}
          onAdd={async (result, facet, options) => {
            const titleId = await addMetadataSearchResultToCatalog(
              result,
              facet,
              options,
            );
            if (titleId) {
              await refresh();
            }
            return titleId;
          }}
        />
      ) : null}
      {!canManageTitle && canRequestMedia ? (
        <RequestMediaDialog
          open={requestDialogOpen}
          onOpenChange={handleRequestDialogOpenChange}
          result={selectedResult}
          facet={selectedFacet}
          requestableLibraries={requestableLibrariesByFacet[selectedFacet] ?? []}
          qualityProfileOptions={catalogQualityProfileOptions}
          onRequest={async (result, facet, options) => {
            const accepted = await requestMetadataSearchResult(
              result,
              facet,
              options,
            );
            if (accepted) {
              await refresh();
            }
            return accepted;
          }}
        />
      ) : null}
    </>
  );
});
