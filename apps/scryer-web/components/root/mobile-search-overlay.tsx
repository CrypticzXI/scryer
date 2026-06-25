import * as React from "react";
import {
  ArrowRight,
  CircleCheck,
  Eraser,
  Eye,
  Info,
  Loader2,
  Plus,
  Search,
  SearchX,
  Send,
  X,
} from "lucide-react";
import { Input } from "@/components/ui/input";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import type { OverviewTitleTarget, ViewId } from "@/components/root/types";
import {
  filterRouteCommandItems,
  groupRouteCommandItems,
  routeCommandDisplayLabel,
  type RouteCommandItem,
} from "@/components/common/route-command-types";
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
  viewAllLabelForFacet,
  viewFromFacet,
} from "@/lib/facets/helpers";
import { useSearchContext } from "@/lib/context/search-context";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";
import {
  globalSearchConfigureAddId,
  globalSearchMetadataResultId,
  globalSearchRequestId,
} from "@/lib/utils/dom-ids";
import { TitlePosterSlot } from "@/components/title-poster-slot";
import {
  AddToCatalogDialog,
  EMPTY_SEARCH_RESULT,
} from "@/components/root/add-to-catalog-dialog";
import { RequestMediaDialog } from "@/components/root/request-media-dialog";

type MobileSearchOverlayProps = {
  canViewCatalog: boolean;
  onClose: () => void;
  routeCommandItems?: RouteCommandItem[];
  onOpenOverview?: (
    targetView: ViewId,
    overviewTarget: OverviewTitleTarget,
  ) => void;
};

type SearchTabKey = "all" | "library" | "actions" | Facet;

function catalogFacetFromString(facet: string): Facet {
  return facet === "movie" ? "movie" : facet === "anime" ? "anime" : "series";
}

function SearchSectionLoading({ label }: { label: string }) {
  return (
    <div className="flex min-h-20 items-center justify-center gap-3 rounded-[12px] border border-dashed border-[var(--scry-border2)] bg-[linear-gradient(180deg,var(--scry-surfC),var(--scry-surfF))] px-4 py-3 text-sm text-[var(--scry-muted3)]">
      <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-[9px] bg-[var(--scry-chip)] text-[var(--scry-accent-ring)]">
        <Loader2 className="h-4 w-4 animate-spin" />
      </span>
      <span className="font-medium">{label}</span>
    </div>
  );
}

export function MobileSearchOverlay({
  canViewCatalog,
  onClose,
  routeCommandItems = [],
  onOpenOverview,
}: MobileSearchOverlayProps) {
  const searchState = useSearchContext();
  const {
    globalSearch,
    globalSearchInputRef,
    catalogSearchResults,
    metadataSearchResults,
    catalogSearchLoading,
    metadataSearchLoading,
    searching,
  } = searchState;
  const t = useTranslate();
  const searchOverlayPlaceholder = canViewCatalog
    ? t("search.overlayPlaceholder")
    : t("search.overlayPlaceholderNoLibrary");
  const searchSubtitle = canViewCatalog
    ? t("search.subtitle")
    : t("search.subtitleNoLibrary");
  const searchMinimumQueryHint = canViewCatalog
    ? t("search.minimumQueryHint")
    : t("search.minimumQueryHintNoLibrary");
  const searchEmptyHint = canViewCatalog
    ? t("search.emptyHint")
    : t("search.emptyHintNoLibrary");
  const searchTipTitles = canViewCatalog
    ? t("search.tipTitles")
    : t("search.tipTitlesNoLibrary");
  const searchTipTabs = canViewCatalog
    ? t("search.tipTabs")
    : t("search.tipTabsNoLibrary");
  const trimmedGlobalSearch = globalSearch.trim();
  const hasMinimumGlobalSearchQuery = trimmedGlobalSearch.length >= 2;
  const overlayRef = React.useRef<HTMLDivElement>(null);
  const inputRef = React.useRef<HTMLInputElement>(null);
  const mobileSearchResultsRef = React.useRef<HTMLDivElement>(null);
  const mobileSearchTabRefs = React.useRef<
    Partial<Record<SearchTabKey, HTMLButtonElement | null>>
  >({});
  const [activeTab, setActiveTab] = React.useState<SearchTabKey>("all");
  const [addDialogTarget, setAddDialogTarget] = React.useState<{
    result: MetadataTvdbSearchItem;
    facet: Facet;
  } | null>(null);
  const [requestDialogTarget, setRequestDialogTarget] = React.useState<{
    result: MetadataTvdbSearchItem;
    facet: Facet;
  } | null>(null);
  const closingAfterSuccessfulActionRef = React.useRef(false);
  const setMobileSearchInputRef = React.useCallback(
    (node: HTMLInputElement | null) => {
      inputRef.current = node;
      globalSearchInputRef.current = node;
    },
    [globalSearchInputRef],
  );

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

  const catalogSearchSections = React.useMemo(() => {
    const results = catalogSearchResults;
    const query = globalSearch.trim().toLowerCase();
    const rank = (title: import("@/lib/types").TitleRecord) => {
      const name = title.name.trim().toLowerCase();
      if (!query || name === query) return 0;
      if (name.startsWith(query)) return 1;
      const matchIndex = name.indexOf(query);
      return matchIndex >= 0 ? 2 + matchIndex : 3 + name.length;
    };

    const buckets = Object.fromEntries(
      FACET_REGISTRY.map((f) => [
        f.id,
        [] as import("@/lib/types").TitleRecord[],
      ]),
    ) as Record<Facet, import("@/lib/types").TitleRecord[]>;
    for (const title of results) {
      const facet = catalogFacetFromString(title.facet);
      buckets[facet].push(title);
    }
    if (query) {
      for (const facet of FACET_REGISTRY) {
        buckets[facet.id].sort((a, b) => rank(a) - rank(b));
      }
    }
    return buckets;
  }, [catalogSearchResults, globalSearch]);

  const metadataResultCounts = React.useMemo(
    () =>
      Object.fromEntries(
        FACET_REGISTRY.map((f) => [
          f.id,
          (metadataSearchResults[f.metadataKey] ?? []).length,
        ]),
      ) as Record<Facet, number>,
    [metadataSearchResults],
  );

  const metadataResultCount = FACET_REGISTRY.reduce(
    (total, f) => total + metadataResultCounts[f.id],
    0,
  );
  const routeCommandResults = React.useMemo(
    () => filterRouteCommandItems(routeCommandItems, globalSearch),
    [globalSearch, routeCommandItems],
  );
  const visibleRouteCommandResults = React.useMemo(
    () =>
      activeTab === "all"
        ? routeCommandResults.slice(0, 6)
        : activeTab === "actions"
          ? routeCommandResults
          : [],
    [activeTab, routeCommandResults],
  );
  const hiddenRouteCommandResultCount = Math.max(
    routeCommandResults.length - visibleRouteCommandResults.length,
    0,
  );
  const visibleCatalogFacets = React.useMemo(() => {
    if (!canViewCatalog || activeTab === "actions") {
      return [];
    }

    return activeTab === "all" || activeTab === "library"
      ? FACET_REGISTRY
      : FACET_REGISTRY.filter((f) => f.id === activeTab);
  }, [activeTab, canViewCatalog]);

  const visibleMetadataFacets = React.useMemo(
    () =>
      activeTab === "library" || activeTab === "actions"
        ? []
        : activeTab === "all"
          ? FACET_REGISTRY
          : FACET_REGISTRY.filter((f) => f.id === activeTab),
    [activeTab],
  );
  const metadataSectionFacets = React.useMemo(
    () =>
      metadataSearchLoading
        ? visibleMetadataFacets
        : visibleMetadataFacets.filter((f) => metadataResultCounts[f.id] > 0),
    [metadataResultCounts, metadataSearchLoading, visibleMetadataFacets],
  );

  const visibleCatalogCount = visibleCatalogFacets.reduce(
    (total, f) => total + catalogSearchSections[f.id].length,
    0,
  );

  const visibleCatalogResults = React.useMemo(() => {
    const picked: import("@/lib/types").TitleRecord[] = [];
    if (!canViewCatalog || activeTab === "actions") {
      return picked;
    }

    if (activeTab !== "all" && activeTab !== "library") {
      return catalogSearchSections[activeTab];
    }

    const indices: Record<string, number> = {};
    const maxItems = activeTab === "all" ? 4 : Number.POSITIVE_INFINITY;
    while (picked.length < maxItems) {
      let added = false;
      for (const f of visibleCatalogFacets) {
        if (picked.length >= maxItems) break;
        const bucket = catalogSearchSections[f.id];
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
  }, [activeTab, canViewCatalog, catalogSearchSections, visibleCatalogFacets]);
  const visibleCatalogResultCount = canViewCatalog
    ? catalogSearchResults.length
    : 0;
  const hiddenCatalogResultCount = Math.max(
    visibleCatalogCount - visibleCatalogResults.length,
    0,
  );

  const mobileSearchTabs = React.useMemo(
    () => [
      {
        key: "all" as SearchTabKey,
        label: t("search.tabAll"),
        count:
          visibleCatalogResultCount +
          metadataResultCount +
          routeCommandResults.length,
      },
      ...(canViewCatalog
        ? [
            {
              key: "library" as SearchTabKey,
              label: t("search.tabLibrary"),
              count: visibleCatalogResultCount,
            },
          ]
        : []),
      ...FACET_REGISTRY.map((f) => ({
        key: f.id as SearchTabKey,
        label: t(f.navLabelKey),
        count:
          (canViewCatalog ? catalogSearchSections[f.id].length : 0) +
          metadataResultCounts[f.id],
      })),
      ...(routeCommandResults.length > 0 || activeTab === "actions"
        ? [
            {
              key: "actions" as SearchTabKey,
              label: t("search.tabActions"),
              count: routeCommandResults.length,
            },
          ]
        : []),
    ],
    [
      activeTab,
      canViewCatalog,
      catalogSearchSections,
      metadataResultCount,
      metadataResultCounts,
      routeCommandResults.length,
      t,
      visibleCatalogResultCount,
    ],
  );
  const searchStatusLabel = React.useMemo(() => {
    const isLoading =
      searching ||
      (canViewCatalog && catalogSearchLoading) ||
      metadataSearchLoading;
    if (!trimmedGlobalSearch) {
      return searchSubtitle;
    }
    if (!hasMinimumGlobalSearchQuery && routeCommandResults.length === 0) {
      return searchMinimumQueryHint;
    }
    if (isLoading) {
      return t("search.statusLoading", { query: trimmedGlobalSearch });
    }

    const resultCount =
      visibleCatalogResultCount +
      metadataResultCount +
      routeCommandResults.length;
    if (resultCount === 0) {
      return t("search.statusNoResults", { query: trimmedGlobalSearch });
    }
    return resultCount === 1
      ? t("search.statusResultOne", { query: trimmedGlobalSearch })
      : t("search.statusResultOther", {
          count: String(resultCount),
          query: trimmedGlobalSearch,
        });
  }, [
    canViewCatalog,
    catalogSearchLoading,
    hasMinimumGlobalSearchQuery,
    metadataResultCount,
    metadataSearchLoading,
    searchMinimumQueryHint,
    searchSubtitle,
    searching,
    routeCommandResults.length,
    t,
    trimmedGlobalSearch,
    visibleCatalogResultCount,
  ]);

  const focusMobileSearchTab = React.useCallback((nextTab: SearchTabKey) => {
    setActiveTab(nextTab);
    const nextTabElement = mobileSearchTabRefs.current[nextTab];
    nextTabElement?.focus();
    nextTabElement?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, []);

  const handleMobileSearchTabKeyDown = React.useCallback(
    (
      event: React.KeyboardEvent<HTMLButtonElement>,
      currentTab: SearchTabKey,
    ) => {
      const tabKeys = mobileSearchTabs.map((tab) => tab.key);
      if (tabKeys.length === 0) {
        return;
      }

      const currentIndex = tabKeys.indexOf(currentTab);
      const safeIndex = currentIndex === -1 ? 0 : currentIndex;
      let nextTab: SearchTabKey | null = null;

      if (event.key === "ArrowRight") {
        nextTab = tabKeys[(safeIndex + 1) % tabKeys.length] ?? null;
      } else if (event.key === "ArrowLeft") {
        nextTab =
          tabKeys[(safeIndex - 1 + tabKeys.length) % tabKeys.length] ?? null;
      } else if (event.key === "Home") {
        nextTab = tabKeys[0] ?? null;
      } else if (event.key === "End") {
        nextTab = tabKeys[tabKeys.length - 1] ?? null;
      }

      if (!nextTab) {
        return;
      }

      event.preventDefault();
      focusMobileSearchTab(nextTab);
    },
    [focusMobileSearchTab, mobileSearchTabs],
  );

  React.useEffect(() => {
    if (!mobileSearchTabs.some((tab) => tab.key === activeTab)) {
      setActiveTab("all");
    }
  }, [activeTab, mobileSearchTabs]);

  React.useEffect(() => {
    mobileSearchResultsRef.current?.scrollTo({ left: 0, top: 0 });
  }, [activeTab, globalSearch]);

  const {
    catalogConfigLoading,
    ensureCatalogConfigReady,
    isCatalogConfigReady,
    resolveDefaultQualityProfileIdForFacet,
    addMetadataSearchResultToCatalog,
    requestMetadataSearchResult,
    isMetadataSearchResultInCatalog,
    catalogQualityProfileOptions,
    librariesByFacet,
    requestableLibrariesByFacet,
    setGlobalSearch,
    clearGlobalSearch,
    resetGlobalSearch,
    forceSearchGlobal,
  } = searchState;
  const isAddDialogConfigReady = addDialogTarget
    ? isCatalogConfigReady(addDialogTarget.facet)
    : true;

  const getMobileSearchResultButtons = React.useCallback(() => {
    const resultRoot = mobileSearchResultsRef.current;
    if (!resultRoot) {
      return [];
    }

    return Array.from(
      resultRoot.querySelectorAll<HTMLButtonElement>(
        "[data-mobile-global-search-result='true']:not(:disabled)",
      ),
    );
  }, []);

  const focusMobileSearchResult = React.useCallback(
    (position: "first" | "last") => {
      const buttons = getMobileSearchResultButtons();
      if (buttons.length === 0) {
        return false;
      }

      buttons[position === "first" ? 0 : buttons.length - 1]?.focus();
      return true;
    },
    [getMobileSearchResultButtons],
  );

  const focusRelativeMobileSearchResult = React.useCallback(
    (currentButton: HTMLButtonElement, delta: 1 | -1) => {
      const buttons = getMobileSearchResultButtons();
      if (buttons.length === 0) {
        return false;
      }

      const currentIndex = buttons.indexOf(currentButton);
      if (currentIndex === -1) {
        buttons[delta > 0 ? 0 : buttons.length - 1]?.focus();
        return true;
      }

      const nextIndex =
        (currentIndex + delta + buttons.length) % buttons.length;
      buttons[nextIndex]?.focus();
      return true;
    },
    [getMobileSearchResultButtons],
  );

  const handleMobileSearchInputKeyDown = React.useCallback(
    (event: React.KeyboardEvent<HTMLInputElement>) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        onClose();
        return;
      }

      if (event.key === "Enter") {
        if (event.nativeEvent.isComposing) {
          return;
        }
        event.preventDefault();
        if (focusMobileSearchResult("first")) {
          return;
        }
        void forceSearchGlobal(event.currentTarget.value);
        return;
      }

      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        const didFocus = focusMobileSearchResult(
          event.key === "ArrowDown" ? "first" : "last",
        );
        if (didFocus) {
          event.preventDefault();
        }
      }
    },
    [focusMobileSearchResult, forceSearchGlobal, onClose],
  );

  const handleMobileSearchResultKeyDown = React.useCallback(
    (event: React.KeyboardEvent<HTMLButtonElement>) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        onClose();
        return;
      }

      if (event.key === "Home" || event.key === "End") {
        const didFocus = focusMobileSearchResult(
          event.key === "Home" ? "first" : "last",
        );
        if (didFocus) {
          event.preventDefault();
        }
        return;
      }

      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        const didFocus = focusRelativeMobileSearchResult(
          event.currentTarget,
          event.key === "ArrowDown" ? 1 : -1,
        );
        if (didFocus) {
          event.preventDefault();
        }
      }
    },
    [focusMobileSearchResult, focusRelativeMobileSearchResult, onClose],
  );

  const isNestedSearchDialogOpen =
    addDialogTarget !== null || requestDialogTarget !== null;

  const handleMobileOverlayKeyDown = React.useCallback(
    (event: React.KeyboardEvent<HTMLDivElement>) => {
      if (isNestedSearchDialogOpen) {
        return;
      }

      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
        return;
      }

      if (event.key !== "Tab") {
        return;
      }

      const overlay = overlayRef.current;
      if (!overlay) {
        return;
      }

      const activeElement = document.activeElement;
      if (
        activeElement instanceof Element &&
        !overlay.contains(activeElement) &&
        activeElement.closest(
          "[data-slot='popover-content'], [data-slot='select-content'], [data-slot='dialog-content']",
        )
      ) {
        return;
      }

      const focusableElements = Array.from(
        overlay.querySelectorAll<HTMLElement>(
          "a[href], button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex='-1'])",
        ),
      ).filter((element) => element.offsetParent !== null);

      if (focusableElements.length === 0) {
        event.preventDefault();
        overlay.focus();
        return;
      }

      const firstElement = focusableElements[0];
      const lastElement = focusableElements[focusableElements.length - 1];

      if (!activeElement || !overlay.contains(activeElement)) {
        event.preventDefault();
        firstElement?.focus();
        return;
      }

      if (event.shiftKey && activeElement === firstElement) {
        event.preventDefault();
        lastElement?.focus();
        return;
      }

      if (!event.shiftKey && activeElement === lastElement) {
        event.preventDefault();
        firstElement?.focus();
      }
    },
    [isNestedSearchDialogOpen, onClose],
  );

  const handleOpenAddDialog = React.useCallback(
    (result: MetadataTvdbSearchItem, facet: Facet) => {
      setAddDialogTarget({ result, facet });
      void ensureCatalogConfigReady(facet);
    },
    [ensureCatalogConfigReady],
  );

  const handleAddDialogSubmit = React.useCallback(
    async (
      result: MetadataTvdbSearchItem,
      facet: Facet,
      options: MetadataCatalogAddOptions,
    ) => {
      const titleId = await addMetadataSearchResultToCatalog(
        result,
        facet,
        options,
      );
      if (titleId) {
        const selectedLibrary = librariesByFacet[facet].find(
          (library) => library.id === options.libraryId,
        );
        resetGlobalSearch();
        closingAfterSuccessfulActionRef.current = true;
        onOpenOverview?.(viewFromFacet(facet), {
          id: titleId,
          slug: result.slug ?? null,
          libraryId: selectedLibrary?.id ?? options.libraryId ?? null,
          librarySlug: selectedLibrary?.slug ?? null,
        });
      }
      return titleId;
    },
    [
      addMetadataSearchResultToCatalog,
      librariesByFacet,
      onOpenOverview,
      resetGlobalSearch,
    ],
  );

  const handleRequestDialogSubmit = React.useCallback(
    async (
      result: MetadataTvdbSearchItem,
      facet: Facet,
      options: MetadataCatalogRequestOptions,
    ) => {
      const accepted = await requestMetadataSearchResult(
        result,
        facet,
        options,
      );
      if (accepted) {
        resetGlobalSearch();
        closingAfterSuccessfulActionRef.current = true;
      }
      return accepted;
    },
    [requestMetadataSearchResult, resetGlobalSearch],
  );

  const restoreMobileSearchInputFocus = React.useCallback(() => {
    if (typeof window === "undefined") {
      return;
    }

    window.requestAnimationFrame(() => inputRef.current?.focus());
  }, [inputRef]);

  const handleAddDialogOpenChange = React.useCallback(
    (open: boolean) => {
      if (open) {
        return;
      }
      setAddDialogTarget(null);
      if (closingAfterSuccessfulActionRef.current) {
        closingAfterSuccessfulActionRef.current = false;
        onClose();
        return;
      }
      restoreMobileSearchInputFocus();
    },
    [onClose, restoreMobileSearchInputFocus],
  );

  const handleRequestDialogOpenChange = React.useCallback(
    (open: boolean) => {
      if (open) {
        return;
      }
      setRequestDialogTarget(null);
      if (closingAfterSuccessfulActionRef.current) {
        closingAfterSuccessfulActionRef.current = false;
        onClose();
        return;
      }
      restoreMobileSearchInputFocus();
    },
    [onClose, restoreMobileSearchInputFocus],
  );

  const handleRouteCommandSelect = React.useCallback(
    (item: RouteCommandItem) => {
      resetGlobalSearch();
      onClose();
      item.onSelect();
    },
    [onClose, resetGlobalSearch],
  );

  const renderRouteCommandItem = React.useCallback(
    (item: RouteCommandItem) => {
      const Icon = item.icon ?? Search;
      const description = item.description.trim();
      const groupLabel = item.groupLabel?.trim() || null;
      const displayLabel = routeCommandDisplayLabel(item);
      const showDescription =
        description.length > 0 && description !== displayLabel.trim();
      const commandLabel = [
        displayLabel,
        showDescription ? description : null,
        groupLabel,
      ]
        .filter(Boolean)
        .join(": ");

      return (
        <button
          key={item.id}
          type="button"
          data-mobile-global-search-result="true"
          className="group relative flex w-full min-w-0 items-center gap-3 overflow-hidden rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-surfA)] p-3 text-left transition active:bg-[var(--scry-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/25"
          onClick={() => handleRouteCommandSelect(item)}
          onKeyDown={handleMobileSearchResultKeyDown}
          aria-label={commandLabel}
          title={commandLabel}
        >
          <span
            aria-hidden="true"
            className="absolute inset-y-3 left-0 w-0.5 rounded-r-full bg-[var(--scry-accent-ring)] opacity-0 transition group-active:opacity-100 group-focus-visible:opacity-100"
          />
          <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-[9px] border border-[rgba(var(--scry-accent-rgb),0.22)] bg-[rgba(var(--scry-accent-rgb),0.14)] text-[var(--scry-accent-text)]">
            <Icon className="h-4 w-4" />
          </span>
          <span className="min-w-0 flex-1">
            <span className="block truncate text-sm font-semibold text-[var(--scry-ink2)]">
              {displayLabel}
            </span>
            {showDescription ? (
              <span className="mt-0.5 block truncate text-xs text-[var(--scry-muted3)]">
                {description}
              </span>
            ) : null}
          </span>
          <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-[8px] text-[var(--scry-faint2)] transition group-active:bg-[var(--scry-chip)] group-active:text-[var(--scry-ink2)]">
            <ArrowRight className="h-4 w-4" />
          </span>
        </button>
      );
    },
    [handleMobileSearchResultKeyDown, handleRouteCommandSelect],
  );

  const renderCatalogItem = React.useCallback(
    (
      title: import("@/lib/types").TitleRecord,
      facet: "movie" | "series" | "anime",
    ) => {
      const targetView: ViewId =
        facet === "series" ? "series" : facet === "anime" ? "anime" : "movies";
      const tvdbId = (title.externalIds ?? [])
        .find((externalId) => externalId.source.toLowerCase() === "tvdb")
        ?.value.trim();
      const posterUrl = selectPosterVariantUrl(title.posterUrl, "w70");
      const libraryLabel =
        title.libraryName?.trim() || sectionLabelForFacet(t, facet);
      const qualityLabel =
        title.currentQualityTier?.trim() || title.qualityTier?.trim() || null;
      const statusLabel = title.contentStatus?.trim() || null;
      const secondaryParts = [
        title.year ? String(title.year) : null,
        statusLabel,
        tvdbId ? `TVDB ${tvdbId}` : null,
      ].filter(Boolean);
      const viewTitleLabel = `${t("search.view")}: ${title.name}`;

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
          data-mobile-global-search-result="true"
          onKeyDown={handleMobileSearchResultKeyDown}
          className="group flex w-full flex-wrap items-center gap-[13px] rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-surfA)] p-2.5 text-left shadow-[0_8px_20px_rgba(0,0,0,0.20)] transition active:bg-[var(--scry-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/25 sm:flex-nowrap"
          aria-label={viewTitleLabel}
          title={viewTitleLabel}
        >
          <div className="relative h-16 w-11 flex-none overflow-hidden rounded-[7px] border border-[var(--scry-border2)] bg-muted">
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
            <div className="pointer-events-none absolute inset-x-0 bottom-0 h-1/2 bg-gradient-to-t from-black/70 to-transparent" />
          </div>
          <div className="min-w-0 flex-1">
            <p className="truncate text-sm font-semibold text-foreground">
              {title.name}
            </p>
            <p className="mt-0.5 truncate text-xs text-muted-foreground">
              {title.monitored
                ? t("search.monitored")
                : t("search.unmonitored")}
              {secondaryParts.length > 0 ? (
                <> · {secondaryParts.join(" · ")}</>
              ) : null}
            </p>
            <div className="mt-2 flex min-w-0 flex-wrap items-center gap-2">
              <span className="max-w-[8rem] truncate rounded-md bg-primary/15 px-2 py-0.5 text-[10px] font-bold uppercase tracking-wide text-primary">
                {libraryLabel}
              </span>
              <span className="inline-flex min-w-0 max-w-[8rem] items-center gap-1 text-[11px] font-medium text-emerald-600 dark:text-emerald-300">
                <CircleCheck className="h-3 w-3 shrink-0" />
                <span className="truncate">
                  {qualityLabel ?? t("search.inLibrary")}
                </span>
              </span>
            </div>
          </div>
          <span className="inline-flex h-[34px] w-full shrink-0 items-center justify-center gap-1.5 rounded-[9px] border border-[var(--scry-bhover2)] bg-[var(--scry-soft3)] px-3 text-[12.5px] font-semibold text-[var(--scry-body)] sm:w-auto sm:justify-start">
            <Eye className="h-3.5 w-3.5" />
            {t("search.view")}
          </span>
        </button>
      );
    },
    [handleMobileSearchResultKeyDown, onOpenOverview, resetGlobalSearch, t],
  );

  const renderMetadataItem = React.useCallback(
    (result: MetadataTvdbSearchItem, facet: "movie" | "series" | "anime") => {
      const isInCatalog = isMetadataSearchResultInCatalog(facet, result);
      const hasCatalogAddConfig =
        catalogQualityProfileOptions.length > 0 &&
        librariesByFacet[facet].length > 0;
      const canAdd = hasCatalogAddConfig;
      const canRequest = requestableLibrariesByFacet[facet].length > 0;
      const opensRequestDialog = !canAdd && canRequest;
      const isUnavailable = !isInCatalog && !canAdd && !canRequest;
      const disabled = isInCatalog || isUnavailable;
      const actionLabel = isInCatalog
        ? t("search.alreadyCataloged")
        : isUnavailable
          ? t("search.unavailable")
          : opensRequestDialog
            ? t("search.request")
            : t("search.configureAdd");
      const actionTitle = `${actionLabel}: ${result.name}`;
      const inlineActionLabel = isInCatalog
        ? t("search.cataloged")
        : isUnavailable
          ? t("search.unavailable")
          : opensRequestDialog
            ? t("search.request")
            : t("search.add");
      const posterUrl = selectPosterVariantUrl(result.posterUrl, "w70");
      const actionId = opensRequestDialog
        ? globalSearchRequestId(facet, result)
        : globalSearchConfigureAddId(facet, result);
      const handleMetadataAction = () => {
        if (disabled) {
          return;
        }

        if (opensRequestDialog) {
          setRequestDialogTarget({ result, facet });
          return;
        }

        handleOpenAddDialog(result, facet);
      };

      return (
        <button
          id={globalSearchMetadataResultId(facet, result)}
          key={`${facet}-${result.tvdbId}-${result.name}`}
          type="button"
          className="group w-[7.25rem] flex-none rounded-[12px] text-left outline-none transition focus-visible:ring-2 focus-visible:ring-primary/35 disabled:cursor-default disabled:opacity-75"
          data-mobile-global-search-result="true"
          onClick={handleMetadataAction}
          onKeyDown={handleMobileSearchResultKeyDown}
          disabled={disabled}
          aria-label={actionTitle}
          title={actionTitle}
        >
          <div className="group relative mb-2 aspect-[2/3] overflow-hidden rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-surfA)] shadow-[0_10px_24px_rgba(2,6,23,0.28)] transition group-active:border-[var(--scry-bhover)] group-active:bg-[var(--scry-hover)]">
            <TitlePosterSlot
              src={posterUrl}
              alt={t("media.posterAlt", { name: result.name })}
              className="h-full w-full object-cover"
              placeholderClassName="flex h-full w-full items-center justify-center text-[10px] text-muted-foreground"
              emptyLabel={t("label.noArt")}
              loading="lazy"
            />
            <div className="pointer-events-none absolute inset-x-0 bottom-0 h-2/3 bg-gradient-to-t from-black/85 via-black/35 to-transparent" />
            <p className="pointer-events-none absolute inset-x-2 bottom-2 line-clamp-2 text-[12px] font-bold leading-tight text-white shadow-black drop-shadow">
              {result.name}
            </p>
            <span
              id={actionId}
              className={
                disabled
                  ? "absolute right-2 top-2 inline-flex h-8 w-8 items-center justify-center rounded-lg bg-accent text-card-foreground shadow-lg"
                  : "absolute right-2 top-2 inline-flex h-8 w-8 items-center justify-center rounded-lg bg-primary text-primary-foreground shadow-lg transition group-hover:bg-primary/90"
              }
              aria-hidden="true"
            >
              {isInCatalog ? (
                <CircleCheck className="h-4 w-4" />
              ) : isUnavailable ? (
                <SearchX className="h-4 w-4" />
              ) : opensRequestDialog ? (
                <Send className="h-4 w-4" />
              ) : (
                <Plus className="h-4 w-4" />
              )}
            </span>
          </div>
          <div className="min-w-0 px-1">
            <div className="flex items-center justify-between gap-2">
              <span className="truncate text-[11px] text-muted-foreground">
                {result.year ? result.year : t("label.yearUnknown")}
              </span>
              <span
                className={
                  disabled
                    ? "inline-flex min-w-0 items-center gap-1 rounded-md text-[11px] font-semibold text-muted-foreground"
                    : "inline-flex min-w-0 items-center gap-1 rounded-md text-[11px] font-semibold text-primary transition group-active:text-primary/80"
                }
              >
                {isInCatalog ? (
                  <CircleCheck className="h-3 w-3 shrink-0" />
                ) : isUnavailable ? (
                  <SearchX className="h-3 w-3 shrink-0" />
                ) : opensRequestDialog ? (
                  <Send className="h-3 w-3 shrink-0" />
                ) : (
                  <Plus className="h-3 w-3 shrink-0" />
                )}
                <span className="truncate">{inlineActionLabel}</span>
              </span>
            </div>
          </div>
        </button>
      );
    },
    [
      handleOpenAddDialog,
      handleMobileSearchResultKeyDown,
      isMetadataSearchResultInCatalog,
      catalogQualityProfileOptions.length,
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
    const visibleItems = activeTab === "all" ? items.slice(0, 6) : items;
    const hiddenItemCount = Math.max(items.length - visibleItems.length, 0);
    const facetConfig = FACET_REGISTRY.find((f) => f.id === facet);
    const facetLabel = facetConfig
      ? t(facetConfig.navLabelKey)
      : sectionLabelForFacet(t, facet);
    const viewAllFacetLabel = viewAllLabelForFacet(t, facet);
    const resultCountLabel =
      items.length === 1
        ? t("search.resultCountOne")
        : t("search.resultCountOther", { count: String(items.length) });
    return (
      <section key={`metadata-${facet}`} className="space-y-3">
        <div className="flex items-baseline justify-between gap-3">
          <div className="flex min-w-0 items-baseline gap-2">
            <h3 className="truncate text-[15px] font-bold text-[var(--scry-ink2)]">
              {facetLabel}
            </h3>
            <span className="shrink-0 text-xs text-[var(--scry-muted3)]">
              {loading ? t("search.metadataSearch") : resultCountLabel}
            </span>
          </div>
          {!loading && hiddenItemCount > 0 ? (
            <button
              type="button"
              className="text-xs font-medium text-[var(--scry-accent-ring)]"
              onMouseDown={(event) => event.preventDefault()}
              onClick={() => focusMobileSearchTab(facet)}
              aria-label={viewAllFacetLabel}
            >
              {viewAllFacetLabel}
            </button>
          ) : null}
        </div>
        {loading ? (
          <SearchSectionLoading label={t("label.loading")} />
        ) : (
          <div className="flex gap-3 overflow-x-auto pb-1 [-ms-overflow-style:none] [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
            {visibleItems.map((result) => renderMetadataItem(result, facet))}
          </div>
        )}
      </section>
    );
  };

  const showCatalogSection =
    canViewCatalog && (catalogSearchLoading || visibleCatalogCount > 0);
  const showMetadataSection =
    activeTab !== "library" && metadataSectionFacets.length > 0;
  const showRouteCommandSection = visibleRouteCommandResults.length > 0;
  const showSectionResults =
    showCatalogSection || showMetadataSection || showRouteCommandSection;

  return (
    <div
      id="mobile-global-search-panel"
      ref={overlayRef}
      data-slot="mobile-global-search-overlay"
      className="fixed inset-0 z-50 flex flex-col items-center bg-[rgba(2,4,10,0.66)] px-3 pb-safe pt-safe-comfort text-foreground backdrop-blur-md"
      role="dialog"
      aria-modal="true"
      aria-label={t("search.title")}
      aria-describedby="mobile-global-search-description"
      tabIndex={-1}
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) {
          onClose();
        }
      }}
      onKeyDown={handleMobileOverlayKeyDown}
    >
      <p id="mobile-global-search-description" className="sr-only">
        {searchSubtitle}
      </p>
      <p
        id="mobile-global-search-status"
        className="sr-only"
        role="status"
        aria-live="polite"
        aria-atomic="true"
      >
        {searchStatusLabel}
      </p>
      <div
        data-slot="mobile-global-search-panel"
        className="flex min-h-0 w-full max-w-[920px] flex-1 flex-col overflow-hidden rounded-[18px] border border-[#1f2c46] bg-[linear-gradient(180deg,var(--scry-soft),#080d1a)] shadow-[0_30px_90px_rgba(0,0,0,0.62)]"
        onMouseDown={(event) => event.stopPropagation()}
      >
        {/* Sticky search header */}
        <div
          data-slot="mobile-global-search-header"
          className="flex items-center gap-[13px] border-b border-[var(--scry-border)] px-[14px] py-3"
        >
          <div className="relative min-w-0 flex-1">
            <Search className="pointer-events-none absolute left-3.5 top-1/2 h-[18px] w-[18px] -translate-y-1/2 text-[var(--scry-accent-ring)]" />
            <Input
              ref={setMobileSearchInputRef}
              value={globalSearch}
              onChange={(e) => setGlobalSearch(e.target.value)}
              onKeyDown={handleMobileSearchInputKeyDown}
              className="h-10 w-full rounded-[11px] border border-[var(--scry-border2)] bg-[var(--scry-bg)] pl-10 pr-10 text-[16px] text-[var(--scry-ink2)] shadow-none placeholder:text-[var(--scry-faint)] focus-visible:border-[var(--scry-accent-ring)] focus-visible:ring-primary/20"
              placeholder={searchOverlayPlaceholder}
              aria-label={searchOverlayPlaceholder}
              aria-controls="mobile-global-search-results-panel"
              aria-describedby="mobile-global-search-description mobile-global-search-status"
              autoFocus
            />
            {globalSearch ? (
              <button
                type="button"
                className="absolute right-2 top-1/2 flex h-[26px] w-[26px] -translate-y-1/2 items-center justify-center rounded-[7px] bg-[var(--scry-kbdbg)] text-[var(--scry-muted2)] transition active:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/25"
                onClick={() => {
                  clearGlobalSearch();
                  inputRef.current?.focus();
                }}
                aria-label={t("label.clear")}
                title={t("label.clear")}
              >
                <Eraser className="h-3.5 w-3.5" />
              </button>
            ) : null}
          </div>
          <kbd
            aria-hidden="true"
            className="flex h-[30px] flex-none items-center rounded-[7px] border border-[var(--scry-kbdbd)] bg-[var(--scry-kbdbg)] px-2 text-[11px] font-medium leading-none text-[var(--scry-faint2)]"
          >
            ESC
          </kbd>
          <button
            type="button"
            onClick={onClose}
            className="flex h-[34px] w-[34px] flex-none items-center justify-center rounded-lg bg-[var(--scry-kbdbg)] text-[var(--scry-muted2)] transition active:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/25"
            aria-label={t("label.close")}
            aria-keyshortcuts="Escape"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <div
          data-slot="mobile-global-search-tabs"
          className="flex gap-2 overflow-x-auto border-b border-[var(--scry-border)] px-[14px] py-[13px] [-ms-overflow-style:none] [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
          role="tablist"
          aria-label={t("search.title")}
        >
          {mobileSearchTabs.map((tab) => (
            <button
              id={`mobile-global-search-tab-${tab.key}`}
              key={tab.key}
              ref={(node) => {
                mobileSearchTabRefs.current[tab.key] = node;
              }}
              type="button"
              role="tab"
              aria-selected={activeTab === tab.key}
              aria-controls="mobile-global-search-results-panel"
              tabIndex={activeTab === tab.key ? 0 : -1}
              className={
                activeTab === tab.key
                  ? "inline-flex h-8 shrink-0 items-center gap-2 rounded-[9px] border border-transparent bg-[var(--scry-accent-grad)] px-3 text-[12.5px] font-semibold text-primary-foreground shadow-[0_8px_18px_rgba(var(--scry-accent-rgb),0.24)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/35"
                  : "inline-flex h-8 shrink-0 items-center gap-2 rounded-[9px] border border-[var(--scry-border2)] bg-[var(--scry-card2)] px-3 text-[12.5px] font-semibold text-[var(--scry-muted2)] transition active:bg-[var(--scry-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/35"
              }
              onMouseDown={(event) => event.preventDefault()}
              onClick={() => setActiveTab(tab.key)}
              onKeyDown={(event) =>
                handleMobileSearchTabKeyDown(event, tab.key)
              }
            >
              {tab.label}
              <span className="font-medium tabular-nums opacity-75">
                {tab.count}
              </span>
            </button>
          ))}
        </div>

        {/* Scrollable results */}
        <div
          ref={mobileSearchResultsRef}
          id="mobile-global-search-results-panel"
          data-slot="mobile-global-search-results"
          role="tabpanel"
          aria-labelledby={`mobile-global-search-tab-${activeTab}`}
          className="min-h-0 flex-1 overflow-y-auto px-[14px] py-[18px] [scrollbar-color:var(--scry-border2)_transparent] [scrollbar-width:thin] [&::-webkit-scrollbar]:w-2.5 [&::-webkit-scrollbar-track]:bg-transparent [&::-webkit-scrollbar-thumb]:rounded-full [&::-webkit-scrollbar-thumb]:border-[3px] [&::-webkit-scrollbar-thumb]:border-transparent [&::-webkit-scrollbar-thumb]:bg-[var(--scry-border2)] [&::-webkit-scrollbar-thumb]:bg-clip-content"
        >
          {showSectionResults ? (
            <div className="space-y-6">
              {showCatalogSection ? (
                <section className="space-y-3">
                  <div className="flex items-baseline justify-between gap-3">
                    <div className="min-w-0">
                      <h3 className="text-[15px] font-bold text-[var(--scry-ink2)]">
                        {t("search.inLibrary")}
                      </h3>
                      <p className="truncate text-xs text-[var(--scry-muted3)]">
                        {t("search.alreadyInCollection")}
                      </p>
                    </div>
                    <div className="flex shrink-0 items-center gap-3">
                      {!catalogSearchLoading &&
                      activeTab !== "library" &&
                      hiddenCatalogResultCount > 0 ? (
                        <button
                          type="button"
                          className="text-xs font-medium text-[var(--scry-accent-ring)]"
                          onMouseDown={(event) => event.preventDefault()}
                          onClick={() => focusMobileSearchTab("library")}
                          aria-label={`${t("search.viewAll")} ${t("search.inLibrary")}`}
                        >
                          {t("search.viewAll")}
                        </button>
                      ) : null}
                      <span className="text-xs font-medium tabular-nums text-[var(--scry-muted3)]">
                        {visibleCatalogCount === 1
                          ? t("search.resultCountOne")
                          : t("search.resultCountOther", {
                              count: String(visibleCatalogCount),
                            })}
                      </span>
                    </div>
                  </div>
                  {catalogSearchLoading ? (
                    <SearchSectionLoading label={t("label.loading")} />
                  ) : visibleCatalogResults.length === 0 ? (
                    <p className="rounded-[12px] border border-dashed border-[var(--scry-border2)] bg-[var(--scry-surfC)] px-4 py-5 text-sm text-[var(--scry-muted3)]">
                      {!hasMinimumGlobalSearchQuery
                        ? searchMinimumQueryHint
                        : t("search.noCatalogMatches")}
                    </p>
                  ) : (
                    <div className="space-y-2">
                      {visibleCatalogResults.map((title) =>
                        renderCatalogItem(
                          title,
                          catalogFacetFromString(title.facet),
                        ),
                      )}
                    </div>
                  )}
                </section>
              ) : null}

              {showMetadataSection ? (
                <div className="space-y-5">
                  {metadataSectionFacets.map((f) =>
                    renderMetadataSection(
                      metadataSearchResults[f.metadataKey] ?? [],
                      f.id,
                      f.metadataKey,
                      metadataSearchLoading,
                    ),
                  )}
                </div>
              ) : null}
              {visibleRouteCommandResults.length > 0 ? (
                <section className="space-y-3">
                  <div className="flex items-baseline justify-between gap-3">
                    <div className="min-w-0">
                      <h3 className="truncate text-[15px] font-bold text-[var(--scry-ink2)]">
                        {t("search.actionsAndSettings")}
                      </h3>
                      <p className="truncate text-xs text-[var(--scry-muted3)]">
                        {routeCommandResults.length === 1
                          ? t("search.resultCountOne")
                          : t("search.resultCountOther", {
                              count: String(routeCommandResults.length),
                            })}
                      </p>
                    </div>
                    {activeTab === "all" &&
                    hiddenRouteCommandResultCount > 0 ? (
                      <button
                        type="button"
                        className="text-xs font-medium text-[var(--scry-accent-ring)]"
                        onMouseDown={(event) => event.preventDefault()}
                        onClick={() => focusMobileSearchTab("actions")}
                      >
                        {t("search.viewAll")}
                      </button>
                    ) : null}
                  </div>
                  <div className="space-y-3">
                    {groupRouteCommandItems(visibleRouteCommandResults).map(
                      (group) => (
                        <div
                          key={group.groupLabel ?? "ungrouped"}
                          className="space-y-2"
                        >
                          {group.groupLabel ? (
                            <p className="text-[11px] font-semibold uppercase tracking-wide text-[var(--scry-muted3)]">
                              {group.groupLabel}
                            </p>
                          ) : null}
                          <div className="space-y-2">
                            {group.items.map(renderRouteCommandItem)}
                          </div>
                        </div>
                      ),
                    )}
                  </div>
                </section>
              ) : null}
              <div className="flex flex-wrap items-center justify-center gap-2 pt-1 text-center text-xs text-[var(--scry-muted3)]">
                <Info className="h-3.5 w-3.5 shrink-0 text-[var(--scry-faint2)]" />
                <span>{t("search.footerTip")}</span>
                <Popover>
                  <PopoverTrigger asChild>
                    <button
                      type="button"
                      className="font-medium text-[var(--scry-accent-ring)] transition hover:text-[var(--scry-accent-text)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/35"
                    >
                      {t("search.searchTips")}
                    </button>
                  </PopoverTrigger>
                  <PopoverContent
                    align="center"
                    sideOffset={8}
                    className="z-[70] w-72 max-w-[calc(100vw-2rem)] border-[var(--scry-border2)] bg-[linear-gradient(180deg,var(--scry-soft),var(--scry-bg))] p-3 text-xs shadow-[0_18px_48px_rgba(0,0,0,0.32)]"
                  >
                    <div className="space-y-2 text-[var(--scry-muted3)]">
                      <p>{searchTipTitles}</p>
                      <p>{searchTipTabs}</p>
                      {canViewCatalog ? <p>{t("search.tipIndexers")}</p> : null}
                    </div>
                  </PopoverContent>
                </Popover>
              </div>
            </div>
          ) : searching ? (
            <div className="py-6">
              <SearchSectionLoading label={t("label.searching")} />
            </div>
          ) : trimmedGlobalSearch ? (
            <div className="flex flex-col items-center justify-center py-12 text-center">
              <div className="mb-4 flex h-14 w-14 items-center justify-center rounded-[15px] border border-[var(--scry-border2)] bg-[var(--scry-chip)] text-[var(--scry-faint2)]">
                {hasMinimumGlobalSearchQuery ? (
                  <SearchX className="h-6 w-6" />
                ) : (
                  <Search className="h-6 w-6" />
                )}
              </div>
              <p className="text-[17px] font-bold text-[var(--scry-ink2)]">
                {!hasMinimumGlobalSearchQuery
                  ? t("search.minimumQueryTitle")
                  : t("search.noMatchesFor", { query: trimmedGlobalSearch })}
              </p>
              <p className="mt-1 max-w-sm text-sm leading-6 text-[var(--scry-muted3)]">
                {!hasMinimumGlobalSearchQuery
                  ? searchMinimumQueryHint
                  : searchEmptyHint}
              </p>
              <Popover>
                <PopoverTrigger asChild>
                  <button
                    type="button"
                    className="mt-3 text-xs font-medium text-[var(--scry-accent-ring)] transition hover:text-[var(--scry-accent-text)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/35"
                  >
                    {t("search.searchTips")}
                  </button>
                </PopoverTrigger>
                <PopoverContent
                  align="center"
                  sideOffset={8}
                  className="z-[70] w-72 max-w-[calc(100vw-2rem)] border-[var(--scry-border2)] bg-[linear-gradient(180deg,var(--scry-soft),var(--scry-bg))] p-3 text-xs shadow-[0_18px_48px_rgba(0,0,0,0.32)]"
                >
                  <div className="space-y-2 text-[var(--scry-muted3)]">
                    <p>{searchTipTitles}</p>
                    <p>{searchTipTabs}</p>
                    {canViewCatalog ? <p>{t("search.tipIndexers")}</p> : null}
                  </div>
                </PopoverContent>
              </Popover>
            </div>
          ) : (
            <div className="flex flex-col items-center justify-center py-12 text-center">
              <div className="mb-4 flex h-14 w-14 items-center justify-center rounded-[15px] border border-[var(--scry-border2)] bg-[var(--scry-chip)] text-[var(--scry-faint2)]">
                <Search className="h-6 w-6" />
              </div>
              <p className="text-[17px] font-bold text-[var(--scry-ink2)]">
                {searchOverlayPlaceholder}
              </p>
              <p className="mt-1 max-w-sm text-sm leading-6 text-[var(--scry-muted3)]">
                {searchEmptyHint}
              </p>
              <Popover>
                <PopoverTrigger asChild>
                  <button
                    type="button"
                    className="mt-3 text-xs font-medium text-[var(--scry-accent-ring)] transition hover:text-[var(--scry-accent-text)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/35"
                  >
                    {t("search.searchTips")}
                  </button>
                </PopoverTrigger>
                <PopoverContent
                  align="center"
                  sideOffset={8}
                  className="z-[70] w-72 max-w-[calc(100vw-2rem)] border-[var(--scry-border2)] bg-[linear-gradient(180deg,var(--scry-soft),var(--scry-bg))] p-3 text-xs shadow-[0_18px_48px_rgba(0,0,0,0.32)]"
                >
                  <div className="space-y-2 text-[var(--scry-muted3)]">
                    <p>{searchTipTitles}</p>
                    <p>{searchTipTabs}</p>
                    {canViewCatalog ? <p>{t("search.tipIndexers")}</p> : null}
                  </div>
                </PopoverContent>
              </Popover>
            </div>
          )}
        </div>
      </div>
      <AddToCatalogDialog
        open={addDialogTarget !== null}
        onOpenChange={handleAddDialogOpenChange}
        result={addDialogTarget?.result ?? EMPTY_SEARCH_RESULT}
        facet={addDialogTarget?.facet ?? "series"}
        catalogQualityProfileOptions={catalogQualityProfileOptions}
        catalogConfigLoading={
          Boolean(addDialogTarget) &&
          catalogConfigLoading &&
          !isAddDialogConfigReady
        }
        defaultQualityProfileId={resolveDefaultQualityProfileIdForFacet(
          addDialogTarget?.facet ?? "series",
        )}
        manageableLibraries={
          librariesByFacet[addDialogTarget?.facet ?? "series"]
        }
        onAdd={handleAddDialogSubmit}
      />
      <RequestMediaDialog
        open={requestDialogTarget !== null}
        onOpenChange={handleRequestDialogOpenChange}
        result={requestDialogTarget?.result ?? EMPTY_SEARCH_RESULT}
        facet={requestDialogTarget?.facet ?? "series"}
        requestableLibraries={
          requestableLibrariesByFacet[requestDialogTarget?.facet ?? "series"]
        }
        qualityProfileOptions={catalogQualityProfileOptions}
        onRequest={handleRequestDialogSubmit}
      />
    </div>
  );
}
