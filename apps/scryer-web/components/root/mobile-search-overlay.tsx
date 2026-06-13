
import * as React from "react";
import { ArrowLeft, Loader2, Plus, Search, Send, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import type { OverviewTitleTarget, ViewId } from "@/components/root/types";
import { useTranslate } from "@/lib/context/translate-context";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import type { Facet } from "@/lib/types";
import type {
  MetadataCatalogAddOptions,
  MetadataCatalogRequestOptions,
} from "@/lib/hooks/use-global-search";
import { FACET_REGISTRY } from "@/lib/facets/registry";
import {
  sectionLabelForFacet,
  viewFromFacet,
} from "@/lib/facets/helpers";
import { useSearchContext } from "@/lib/context/search-context";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";
import {
  globalSearchConfigureAddId,
  globalSearchMetadataResultId,
} from "@/lib/utils/dom-ids";
import { TitlePosterSlot } from "@/components/title-poster-slot";
import { AddToCatalogDialog, EMPTY_SEARCH_RESULT } from "@/components/root/add-to-catalog-dialog";
import { RequestMediaDialog } from "@/components/root/request-media-dialog";

type MobileSearchOverlayProps = {
  onClose: () => void;
  onOpenOverview?: (targetView: ViewId, overviewTarget: OverviewTitleTarget) => void;
};

function catalogFacetFromString(facet: string): Facet {
  return facet === "movie" ? "movie" : facet === "anime" ? "anime" : "series";
}

function SearchSectionLoading({ label }: { label: string }) {
  return (
    <div className="flex min-h-20 items-center gap-3 rounded-lg border border-dashed border-border/80 bg-muted/30 px-4 py-3 text-sm text-muted-foreground">
      <Loader2 className="h-4 w-4 animate-spin text-emerald-500" />
      <span>{label}</span>
    </div>
  );
}

export function MobileSearchOverlay({
  onClose,
  onOpenOverview,
}: MobileSearchOverlayProps) {
  const searchState = useSearchContext();
  const t = useTranslate();
  const inputRef = React.useRef<HTMLInputElement>(null);
  const [addDialogTarget, setAddDialogTarget] = React.useState<{
    result: MetadataTvdbSearchItem;
    facet: Facet;
  } | null>(null);
  const [requestDialogTarget, setRequestDialogTarget] = React.useState<{
    result: MetadataTvdbSearchItem;
    facet: Facet;
  } | null>(null);

  // Focus the input when the overlay mounts.
  // Mobile Safari restricts focus() to user-gesture contexts, so we also
  // use autoFocus on the input and retry with a short delay as a fallback.
  React.useEffect(() => {
    inputRef.current?.focus();
    const timer = setTimeout(() => inputRef.current?.focus(), 50);
    return () => clearTimeout(timer);
  }, []);

  // Prevent body scroll while overlay is open
  React.useEffect(() => {
    const original = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = original;
    };
  }, []);

  const topCatalogResults = React.useMemo(() => {
    const results = searchState.catalogSearchResults;
    if (results.length === 0) return results;

    const query = searchState.globalSearch.trim().toLowerCase();
    const rank = (title: import("@/lib/types").TitleRecord) => {
      const name = title.name.toLowerCase();
      if (name === query) return 0;
      if (name.startsWith(query)) return 1;
      return 2 + name.length;
    };

    const buckets: Record<string, import("@/lib/types").TitleRecord[]> = {};
    for (const title of results) {
      const facet = catalogFacetFromString(title.facet);
      (buckets[facet] ??= []).push(title);
    }
    for (const key of Object.keys(buckets)) {
      buckets[key].sort((a, b) => rank(a) - rank(b));
    }

    // Round-robin: take one from each facet in registry order, repeat
    const picked: import("@/lib/types").TitleRecord[] = [];
    const indices: Record<string, number> = {};
    while (picked.length < 3) {
      let added = false;
      for (const f of FACET_REGISTRY) {
        if (picked.length >= 3) break;
        const bucket = buckets[f.id];
        if (!bucket) continue;
        const idx = indices[f.id] ?? 0;
        if (idx < bucket.length) {
          picked.push(bucket[idx]);
          indices[f.id] = idx + 1;
          added = true;
        }
      }
      if (!added) break;
    }
    return picked;
  }, [searchState.catalogSearchResults, searchState.globalSearch]);

  const hasMetadataMatches = FACET_REGISTRY.some(
    (f) => (searchState.metadataSearchResults[f.metadataKey] ?? []).length > 0,
  );


  const {
    catalogConfigLoading,
    ensureCatalogConfigReady,
    isCatalogConfigReady,
    resolveDefaultQualityProfileIdForFacet,
    addMetadataSearchResultToCatalog,
    requestMetadataSearchResult,
    isMetadataSearchResultInCatalog,
    catalogQualityProfileOptions,
    rootFoldersByFacet,
    librariesByFacet,
    requestableLibrariesByFacet,
    setGlobalSearch,
    resetGlobalSearch,
  } = searchState;
  const isAddDialogConfigReady = addDialogTarget
    ? isCatalogConfigReady(addDialogTarget.facet)
    : true;

  const handleOpenAddDialog = React.useCallback(
    (result: MetadataTvdbSearchItem, facet: Facet) => {
      setAddDialogTarget({ result, facet });
      void ensureCatalogConfigReady(facet);
    },
    [ensureCatalogConfigReady],
  );

  const handleAddDialogSubmit = React.useCallback(
    async (result: MetadataTvdbSearchItem, facet: Facet, options: MetadataCatalogAddOptions) => {
      const titleId = await addMetadataSearchResultToCatalog(result, facet, options);
      if (titleId) {
        const selectedLibrary = librariesByFacet[facet].find((library) => library.id === options.libraryId);
        resetGlobalSearch();
        onOpenOverview?.(viewFromFacet(facet), {
          id: titleId,
          slug: result.slug ?? null,
          libraryId: selectedLibrary?.id ?? options.libraryId ?? null,
          librarySlug: selectedLibrary?.slug ?? null,
        });
      }
      return titleId;
    },
    [addMetadataSearchResultToCatalog, librariesByFacet, onOpenOverview, resetGlobalSearch],
  );

  const handleRequestDialogSubmit = React.useCallback(
    async (result: MetadataTvdbSearchItem, facet: Facet, options: MetadataCatalogRequestOptions) => {
      const accepted = await requestMetadataSearchResult(result, facet, options);
      if (accepted) {
        resetGlobalSearch();
      }
      return accepted;
    },
    [requestMetadataSearchResult, resetGlobalSearch],
  );

  const renderCatalogItem = React.useCallback(
    (title: import("@/lib/types").TitleRecord, facet: "movie" | "series" | "anime") => {
      const targetView: ViewId =
        facet === "series" ? "series" : facet === "anime" ? "anime" : "movies";
      const tvdbId = (title.externalIds ?? [])
        .find((externalId) => externalId.source.toLowerCase() === "tvdb")
        ?.value.trim();
      const posterUrl = selectPosterVariantUrl(title.posterUrl, "w70");

      return (
        <button
          key={title.id}
          type="button"
          onClick={() => {
            resetGlobalSearch();
            onOpenOverview?.(targetView, {
              id: title.id,
              slug: title.slug ?? null,
              libraryId: title.libraryId,
              librarySlug: title.librarySlug ?? null,
            });
          }}
          className="block w-full rounded-lg border border-border bg-card/60 p-3 text-left active:bg-accent/80"
          aria-label={title.name}
        >
          <div className="flex min-h-[44px] items-center gap-3">
            <div className="h-16 w-11 flex-none overflow-hidden rounded-md border border-border bg-muted">
              <TitlePosterSlot
                src={posterUrl}
                sourceSrc={title.posterSourceUrl}
                metadataFetchedAt={title.metadataFetchedAt}
                createdAt={title.createdAt}
                alt={t("media.posterAlt", { name: title.name })}
                className="h-full w-full object-cover"
                placeholderClassName="flex h-full w-full items-center justify-center text-[10px] text-muted-foreground"
                emptyLabel={t("label.noArt")}
                loading="lazy"
              />
            </div>
            <div className="min-w-0 flex-1">
              <p className="text-sm font-medium text-foreground">{title.name}</p>
              <p className="text-xs text-muted-foreground">
                {sectionLabelForFacet(t, facet)} {title.monitored ? `• ${t("label.yes")}` : ""}
                {tvdbId ? ` • ${tvdbId}` : ""}
              </p>
            </div>
          </div>
        </button>
      );
    },
    [onOpenOverview, resetGlobalSearch, t],
  );

  const renderMetadataItem = React.useCallback(
    (result: MetadataTvdbSearchItem, facet: "movie" | "series" | "anime") => {
      const isInCatalog = isMetadataSearchResultInCatalog(facet, result);
      const canAdd = librariesByFacet[facet].length > 0;
      const canRequest = requestableLibrariesByFacet[facet].length > 0;
      const opensRequestDialog = !canAdd && canRequest;
      const disabled = isInCatalog || (!canAdd && !canRequest);
      const actionLabel = isInCatalog
        ? t("search.alreadyCataloged")
        : opensRequestDialog
          ? t("search.request")
          : t("search.configureAdd");
      const posterUrl = selectPosterVariantUrl(result.posterUrl, "w70");

      return (
        <div
          id={globalSearchMetadataResultId(facet, result)}
          key={`${facet}-${result.tvdbId}-${result.name}`}
          className="rounded-lg border border-border bg-card/60 p-3"
        >
          <div className="flex min-h-[44px] items-center gap-3">
            <div className="h-16 w-11 flex-none overflow-hidden rounded-md border border-border bg-muted">
              <TitlePosterSlot
                src={posterUrl}
                alt={t("media.posterAlt", { name: result.name })}
                className="h-full w-full object-cover"
                placeholderClassName="flex h-full w-full items-center justify-center text-[10px] text-muted-foreground"
                emptyLabel={t("label.noArt")}
                loading="lazy"
              />
            </div>
            <div className="min-w-0 flex-1">
              <p className="text-sm font-medium text-foreground">{result.name}</p>
              <p className="text-xs text-muted-foreground">
                {result.type || t("label.unknownType")} • {result.year || t("label.yearUnknown")}
              </p>
            </div>
            <Button
              id={globalSearchConfigureAddId(facet, result)}
              type="button"
              variant={disabled ? "secondary" : "default"}
              className={
                disabled
                  ? "h-10 w-10 flex-none bg-accent text-card-foreground px-0"
                  : "h-10 w-10 flex-none bg-emerald-500 text-foreground hover:bg-emerald-600 px-0"
              }
              onClick={() =>
                opensRequestDialog
                  ? setRequestDialogTarget({ result, facet })
                  : handleOpenAddDialog(result, facet)
              }
              disabled={disabled}
              aria-label={actionLabel}
            >
              {opensRequestDialog ? (
                <Send className="h-4 w-4" />
              ) : (
                <Plus className="h-4 w-4" />
              )}
            </Button>
          </div>

          {result.overview ? (
            <p className="mt-2 text-xs text-muted-foreground line-clamp-2">{result.overview}</p>
          ) : null}
        </div>
      );
    },
    [
      handleOpenAddDialog,
      isMetadataSearchResultInCatalog,
      librariesByFacet,
      requestableLibrariesByFacet,
      t,
    ],
  );

  const renderMetadataSection = (
    items: MetadataTvdbSearchItem[],
    facet: Facet,
    _section: string,
    loading: boolean,
  ) => {
    if (!loading && items.length === 0) return null;
    return (
      <div className="space-y-2">
        <h4 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          {sectionLabelForFacet(t, facet)}
        </h4>
        {loading ? (
          <SearchSectionLoading label={t("label.loading")} />
        ) : (
          <div className="space-y-2">
            {items.slice(0, 3).map((result) => renderMetadataItem(result, facet))}
          </div>
        )}
      </div>
    );
  };

  const showCatalogSection = searchState.catalogSearchLoading || searchState.catalogSearchResults.length > 0;
  const showMetadataSection = searchState.metadataSearchLoading || hasMetadataMatches;
  const showSectionResults = showCatalogSection || showMetadataSection;

  return (
    <div className="fixed inset-0 z-50 flex flex-col bg-background">
      {/* Sticky search header */}
      <div className="flex items-center gap-2 border-b border-border bg-background px-3 pt-safe-comfort py-3 pb-safe">
        <button
          type="button"
          onClick={onClose}
          className="flex h-10 w-10 flex-none items-center justify-center rounded-lg text-muted-foreground active:bg-accent"
          aria-label={t("label.back")}
        >
          <ArrowLeft className="h-5 w-5" />
        </button>
        <div className="relative flex-1">
          <Search className="pointer-events-none absolute left-3 top-1/2 h-5 w-5 -translate-y-1/2 text-muted-foreground" />
          <Input
            ref={inputRef}
            value={searchState.globalSearch}
            onChange={(e) => setGlobalSearch(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                void searchState.forceSearchGlobal();
              }
            }}
            className="h-10 w-full border-emerald-500/70 pl-10 text-base placeholder-heading-font focus-visible:border-emerald-400 focus-visible:ring-emerald-400/45"
            placeholder={t("search.globalPlaceholder")}
            aria-label={t("search.globalPlaceholder")}
            autoFocus
          />
          {searchState.globalSearch ? (
            <button
              type="button"
              className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground"
              onClick={() => {
                setGlobalSearch("");
                inputRef.current?.focus();
              }}
              aria-label={t("label.clear")}
            >
              <X className="h-5 w-5" />
            </button>
          ) : null}
        </div>
      </div>


      {/* Scrollable results */}
      <div className="flex-1 overflow-y-auto px-3 py-4 pb-safe">
        {showSectionResults ? (
          <div className="space-y-6">
            {showCatalogSection ? (
              <section className="space-y-3">
                <h3 className="text-sm font-semibold text-foreground">{t("search.catalog")}</h3>
                {searchState.catalogSearchLoading ? (
                  <SearchSectionLoading label={t("label.loading")} />
                ) : (
                  <div className="space-y-2">
                    {topCatalogResults.map((title) =>
                      renderCatalogItem(title, catalogFacetFromString(title.facet)),
                    )}
                  </div>
                )}
              </section>
            ) : null}

            {showMetadataSection ? (
              <section className="space-y-3">
                <h3 className="text-sm font-semibold text-foreground">{t("search.metadataSearch")}</h3>
                <div className="space-y-3">
                  {FACET_REGISTRY.map((f) =>
                    renderMetadataSection(
                      searchState.metadataSearchResults[f.metadataKey] ?? [],
                      f.id,
                      f.metadataKey,
                      searchState.metadataSearchLoading,
                    ),
                  )}
                </div>
              </section>
            ) : null}
          </div>
        ) : searchState.searching ? (
          <div className="flex items-center gap-3 py-6">
            <Loader2 className="h-5 w-5 animate-spin text-emerald-500" />
            <p className="text-sm text-muted-foreground">{t("label.searching")}</p>
          </div>
        ) : searchState.globalSearch ? (
          <p className="py-6 text-center text-sm text-muted-foreground">{t("status.nothingFound")}</p>
        ) : (
          <p className="py-6 text-center text-sm text-muted-foreground">{t("search.globalPlaceholder")}</p>
        )}
      </div>
      <AddToCatalogDialog
        open={addDialogTarget !== null}
        onOpenChange={(open) => { if (!open) setAddDialogTarget(null); }}
        result={addDialogTarget?.result ?? EMPTY_SEARCH_RESULT}
        facet={addDialogTarget?.facet ?? "series"}
        catalogQualityProfileOptions={catalogQualityProfileOptions}
        catalogConfigLoading={Boolean(addDialogTarget) && catalogConfigLoading && !isAddDialogConfigReady}
        defaultQualityProfileId={resolveDefaultQualityProfileIdForFacet(addDialogTarget?.facet ?? "series")}
        rootFolders={rootFoldersByFacet[addDialogTarget?.facet ?? "series"]}
        manageableLibraries={librariesByFacet[addDialogTarget?.facet ?? "series"]}
        onAdd={handleAddDialogSubmit}
      />
      <RequestMediaDialog
        open={requestDialogTarget !== null}
        onOpenChange={(open) => { if (!open) setRequestDialogTarget(null); }}
        result={requestDialogTarget?.result ?? EMPTY_SEARCH_RESULT}
        facet={requestDialogTarget?.facet ?? "series"}
        requestableLibraries={requestableLibrariesByFacet[requestDialogTarget?.facet ?? "series"]}
        qualityProfileOptions={catalogQualityProfileOptions}
        onRequest={handleRequestDialogSubmit}
      />
    </div>
  );
}
