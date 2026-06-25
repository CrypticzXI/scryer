import * as React from "react";
import {
  ArrowRight,
  ChevronsUpDown,
  CircleCheck,
  Eraser,
  Eye,
  Info,
  Loader2,
  LogOut,
  Plus,
  Search,
  SearchX,
  Send,
  User,
  X,
} from "lucide-react";
import { createPortal } from "react-dom";
import { useNavigate } from "react-router-dom";
import { Button } from "@/components/ui/button";
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
import { useAuth } from "@/lib/hooks/use-auth";
import {
  ROOT_SHELL_MOBILE_BREAKPOINT,
  useIsMobile,
} from "@/lib/hooks/use-mobile";
import { MobileSearchOverlay } from "@/components/root/mobile-search-overlay";
import { FACET_REGISTRY } from "@/lib/facets/registry";
import {
  sectionLabelForFacet,
  viewAllLabelForFacet,
  viewFromFacet,
} from "@/lib/facets/helpers";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";
import { TitlePosterSlot } from "@/components/title-poster-slot";
import { useSearchContext } from "@/lib/context/search-context";
import { cn } from "@/lib/utils";
import { buildViewPath } from "@/lib/utils/routing";
import {
  APP_PERMISSIONS,
  LIBRARY_PERMISSIONS,
  hasAnyAppPermission,
  hasAnyLibraryPermission,
} from "@/lib/utils/permissions";
import {
  globalSearchConfigureAddId,
  globalSearchMetadataResultId,
  globalSearchRequestId,
  selectorId,
} from "@/lib/utils/dom-ids";
import {
  AddToCatalogDialog,
  EMPTY_SEARCH_RESULT,
} from "@/components/root/add-to-catalog-dialog";
import { RequestMediaDialog } from "@/components/root/request-media-dialog";

type RootHeaderProps = {
  mobileNavigation?: React.ReactNode;
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
    <div className="flex min-h-24 items-center justify-center gap-3 rounded-[12px] border border-dashed border-[var(--scry-border2)] bg-[linear-gradient(180deg,var(--scry-surfC),var(--scry-surfF))] px-4 py-3 text-sm text-[var(--scry-muted3)]">
      <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-[9px] bg-[var(--scry-chip)] text-[var(--scry-accent-ring)]">
        <Loader2 className="h-4 w-4 animate-spin" />
      </span>
      <span className="font-medium">{label}</span>
    </div>
  );
}

export const RootHeader = React.memo(function RootHeader({
  mobileNavigation,
  routeCommandItems = [],
  onOpenOverview,
}: RootHeaderProps) {
  const searchState = useSearchContext();
  const {
    resolveDefaultQualityProfileIdForFacet,
    addMetadataSearchResultToCatalog,
    requestMetadataSearchResult,
    clearGlobalSearch,
    resetGlobalSearch,
    openGlobalSearchPanel,
    closeGlobalSearchPanel,
    forceSearchGlobal,
    setGlobalSearch,
    globalSearchInputRef,
    isMetadataSearchResultInCatalog,
    catalogQualityProfileOptions,
    catalogConfigLoading,
    ensureCatalogConfigReady,
    isCatalogConfigReady,
    librariesByFacet,
    requestableLibrariesByFacet,
    catalogSearchResults,
    catalogSearchLoading,
    metadataSearchResults,
    metadataSearchLoading,
    isGlobalSearchPanelOpen,
    globalSearch,
    searching,
  } = searchState;
  const t = useTranslate();
  const isMobile = useIsMobile(ROOT_SHELL_MOBILE_BREAKPOINT);
  const navigate = useNavigate();
  const { token, user, logout, effectiveFormLoginEnabled } = useAuth();
  const canViewCatalog = user
    ? hasAnyLibraryPermission(user, LIBRARY_PERMISSIONS.view)
    : false;
  const searchTriggerPlaceholder = canViewCatalog
    ? t("search.globalPlaceholder")
    : t("search.globalPlaceholderNoLibrary");
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
  const searchShortcutHint = React.useMemo(() => {
    if (typeof navigator === "undefined") {
      return t("search.shortcutHint");
    }
    const platformSignal =
      `${navigator.platform} ${navigator.userAgent}`.toLowerCase();
    return platformSignal.includes("mac") ||
      platformSignal.includes("iphone") ||
      platformSignal.includes("ipad") ||
      platformSignal.includes("ipod")
      ? t("search.shortcutHint")
      : t("search.shortcutHintControl");
  }, [t]);
  const headerRef = React.useRef<HTMLElement>(null);
  const searchShellRef = React.useRef<HTMLDivElement>(null);
  const searchTriggerRef = React.useRef<HTMLButtonElement>(null);
  const searchPanelRef = React.useRef<HTMLDivElement>(null);
  const searchResultsRef = React.useRef<HTMLDivElement>(null);
  const wasGlobalSearchPanelOpenRef = React.useRef(isGlobalSearchPanelOpen);
  const desktopSearchTabRefs = React.useRef<
    Partial<Record<SearchTabKey, HTMLButtonElement | null>>
  >({});
  const lastScrollYRef = React.useRef(0);
  const [accountMenuOpen, setAccountMenuOpen] = React.useState(false);
  const [desktopSearchTab, setDesktopSearchTab] =
    React.useState<SearchTabKey>("all");
  const [mobileHeaderHeight, setMobileHeaderHeight] = React.useState(0);
  const [isMobileHeaderVisible, setIsMobileHeaderVisible] =
    React.useState(true);
  const readPageScrollTop = React.useCallback(() => {
    if (typeof window === "undefined") {
      return 0;
    }

    return Math.max(
      window.scrollY,
      document.scrollingElement?.scrollTop ?? 0,
      document.documentElement?.scrollTop ?? 0,
      document.body?.scrollTop ?? 0,
    );
  }, []);
  const catalogSearchSections = React.useMemo(() => {
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
    for (const title of catalogSearchResults) {
      buckets[catalogFacetFromString(title.facet)].push(title);
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
      desktopSearchTab === "all"
        ? routeCommandResults.slice(0, 6)
        : desktopSearchTab === "actions"
          ? routeCommandResults
          : [],
    [desktopSearchTab, routeCommandResults],
  );
  const hiddenRouteCommandResultCount = Math.max(
    routeCommandResults.length - visibleRouteCommandResults.length,
    0,
  );
  const visibleCatalogFacets = React.useMemo(() => {
    if (!canViewCatalog || desktopSearchTab === "actions") {
      return [];
    }

    return desktopSearchTab === "all" || desktopSearchTab === "library"
      ? FACET_REGISTRY
      : FACET_REGISTRY.filter((f) => f.id === desktopSearchTab);
  }, [canViewCatalog, desktopSearchTab]);
  const visibleMetadataFacets = React.useMemo(
    () =>
      desktopSearchTab === "library" || desktopSearchTab === "actions"
        ? []
        : desktopSearchTab === "all"
          ? FACET_REGISTRY
          : FACET_REGISTRY.filter((f) => f.id === desktopSearchTab),
    [desktopSearchTab],
  );
  const metadataSectionFacets = React.useMemo(
    () =>
      metadataSearchLoading
        ? visibleMetadataFacets
        : visibleMetadataFacets.filter((f) => metadataResultCounts[f.id] > 0),
    [metadataResultCounts, metadataSearchLoading, visibleMetadataFacets],
  );
  const visibleCatalogCount = visibleCatalogFacets.reduce(
    (total, f) => total + (catalogSearchSections[f.id]?.length ?? 0),
    0,
  );
  const visibleCatalogResultCount = canViewCatalog
    ? catalogSearchResults.length
    : 0;
  const visibleCatalogResults = React.useMemo(() => {
    const picked: Array<{
      facet: Facet;
      title: import("@/lib/types").TitleRecord;
    }> = [];
    if (!canViewCatalog || desktopSearchTab === "actions") {
      return picked;
    }

    if (desktopSearchTab !== "all" && desktopSearchTab !== "library") {
      return (catalogSearchSections[desktopSearchTab] ?? []).map((title) => ({
        facet: desktopSearchTab,
        title,
      }));
    }

    const indices: Record<string, number> = {};
    const maxItems = desktopSearchTab === "all" ? 6 : Number.POSITIVE_INFINITY;
    while (picked.length < maxItems) {
      let added = false;
      for (const f of visibleCatalogFacets) {
        if (picked.length >= maxItems) break;
        const bucket = catalogSearchSections[f.id] ?? [];
        const idx = indices[f.id] ?? 0;
        if (idx < bucket.length) {
          picked.push({ facet: f.id, title: bucket[idx] });
          indices[f.id] = idx + 1;
          added = true;
        }
      }
      if (!added) break;
    }
    return picked;
  }, [
    canViewCatalog,
    catalogSearchSections,
    desktopSearchTab,
    visibleCatalogFacets,
  ]);
  const hiddenCatalogResultCount = Math.max(
    visibleCatalogCount - visibleCatalogResults.length,
    0,
  );
  const showCatalogSection =
    canViewCatalog && (catalogSearchLoading || visibleCatalogCount > 0);
  const showRouteCommandSection = visibleRouteCommandResults.length > 0;
  const showSectionResults =
    showCatalogSection ||
    metadataSectionFacets.length > 0 ||
    showRouteCommandSection;
  const desktopSearchTabs = React.useMemo(
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
          (canViewCatalog ? (catalogSearchSections[f.id]?.length ?? 0) : 0) +
          metadataResultCounts[f.id],
      })),
      ...(routeCommandResults.length > 0 || desktopSearchTab === "actions"
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
      canViewCatalog,
      catalogSearchSections,
      desktopSearchTab,
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
  const [addDialogTarget, setAddDialogTarget] = React.useState<{
    result: MetadataTvdbSearchItem;
    facet: Facet;
  } | null>(null);
  const [requestDialogTarget, setRequestDialogTarget] = React.useState<{
    result: MetadataTvdbSearchItem;
    facet: Facet;
  } | null>(null);
  const closingAddDialogAfterSuccessfulActionRef = React.useRef(false);
  const closingRequestDialogAfterSuccessfulActionRef = React.useRef(false);
  const isAddDialogConfigReady = addDialogTarget
    ? isCatalogConfigReady(addDialogTarget.facet)
    : true;

  const handleOpenAddDialog = React.useCallback(
    (result: MetadataTvdbSearchItem, facet: Facet) => {
      closeGlobalSearchPanel();
      setAddDialogTarget({ result, facet });
      void ensureCatalogConfigReady(facet);
    },
    [closeGlobalSearchPanel, ensureCatalogConfigReady],
  );

  const handleOpenRequestDialog = React.useCallback(
    (result: MetadataTvdbSearchItem, facet: Facet) => {
      closeGlobalSearchPanel();
      setRequestDialogTarget({ result, facet });
    },
    [closeGlobalSearchPanel],
  );

  const reopenGlobalSearchAfterDialogCancel = React.useCallback(() => {
    if (isMobile || typeof window === "undefined") {
      return;
    }

    openGlobalSearchPanel(true);
    window.requestAnimationFrame(() => {
      globalSearchInputRef.current?.focus();
      globalSearchInputRef.current?.select();
    });
  }, [globalSearchInputRef, isMobile, openGlobalSearchPanel]);

  const handleAddDialogOpenChange = React.useCallback(
    (open: boolean) => {
      if (open) {
        return;
      }
      setAddDialogTarget(null);
      if (closingAddDialogAfterSuccessfulActionRef.current) {
        closingAddDialogAfterSuccessfulActionRef.current = false;
        return;
      }
      reopenGlobalSearchAfterDialogCancel();
    },
    [reopenGlobalSearchAfterDialogCancel],
  );

  const handleRequestDialogOpenChange = React.useCallback(
    (open: boolean) => {
      if (open) {
        return;
      }
      setRequestDialogTarget(null);
      if (closingRequestDialogAfterSuccessfulActionRef.current) {
        closingRequestDialogAfterSuccessfulActionRef.current = false;
        return;
      }
      reopenGlobalSearchAfterDialogCancel();
    },
    [reopenGlobalSearchAfterDialogCancel],
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
        globalSearchInputRef.current?.blur();
        closingAddDialogAfterSuccessfulActionRef.current = true;
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
      globalSearchInputRef,
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
        globalSearchInputRef.current?.blur();
        closingRequestDialogAfterSuccessfulActionRef.current = true;
      }
      return accepted;
    },
    [globalSearchInputRef, requestMetadataSearchResult, resetGlobalSearch],
  );

  const handleRouteCommandSelect = React.useCallback(
    (item: RouteCommandItem) => {
      resetGlobalSearch();
      globalSearchInputRef.current?.blur();
      item.onSelect();
    },
    [globalSearchInputRef, resetGlobalSearch],
  );

  const handleOpenProfile = React.useCallback(() => {
    setAccountMenuOpen(false);
    navigate(buildViewPath("settings", "profile"));
  }, [navigate]);

  const handleLogout = React.useCallback(() => {
    setAccountMenuOpen(false);
    logout();
    navigate("/login", { replace: true });
  }, [logout, navigate]);

  const handleSearchChange = React.useCallback(
    (event: React.ChangeEvent<HTMLInputElement>) => {
      setGlobalSearch(event.target.value);
      openGlobalSearchPanel(isMobile || undefined);
    },
    [isMobile, openGlobalSearchPanel, setGlobalSearch],
  );

  const restoreSearchTriggerFocus = React.useCallback(() => {
    if (typeof window === "undefined") {
      return;
    }

    if (isMobile) {
      window.requestAnimationFrame(() => searchTriggerRef.current?.focus());
      return;
    }

    const focusTrigger = () => {
      const activeElement = document.activeElement;
      if (
        activeElement &&
        activeElement !== document.body &&
        !searchPanelRef.current?.contains(activeElement)
      ) {
        return;
      }
      searchTriggerRef.current?.focus();
    };

    window.requestAnimationFrame(() => {
      focusTrigger();
      window.setTimeout(focusTrigger, 0);
      window.setTimeout(focusTrigger, 100);
    });
  }, [isMobile]);

  const handleSearchEscape = React.useCallback(
    (event: React.KeyboardEvent<HTMLInputElement>) => {
      if (event.key !== "Escape") {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      resetGlobalSearch();
      globalSearchInputRef.current?.blur();
      restoreSearchTriggerFocus();
    },
    [globalSearchInputRef, resetGlobalSearch, restoreSearchTriggerFocus],
  );

  const handleClearSearch = React.useCallback(() => {
    clearGlobalSearch();
    globalSearchInputRef.current?.focus();
  }, [clearGlobalSearch, globalSearchInputRef]);

  const handleCloseGlobalSearchPanel = React.useCallback(() => {
    resetGlobalSearch();
    globalSearchInputRef.current?.blur();
    restoreSearchTriggerFocus();
  }, [globalSearchInputRef, resetGlobalSearch, restoreSearchTriggerFocus]);

  React.useEffect(() => {
    const wasOpen = wasGlobalSearchPanelOpenRef.current;
    wasGlobalSearchPanelOpenRef.current = isGlobalSearchPanelOpen;
    if (
      wasOpen &&
      !isGlobalSearchPanelOpen &&
      addDialogTarget === null &&
      requestDialogTarget === null
    ) {
      restoreSearchTriggerFocus();
    }
  }, [
    addDialogTarget,
    isGlobalSearchPanelOpen,
    requestDialogTarget,
    restoreSearchTriggerFocus,
  ]);

  const handleSearchPanelKeyDown = React.useCallback(
    (event: React.KeyboardEvent<HTMLDivElement>) => {
      if (event.key !== "Tab") {
        return;
      }

      const panel = searchPanelRef.current;
      if (!panel) {
        return;
      }

      const activeElement = document.activeElement;
      if (
        activeElement instanceof Element &&
        !panel.contains(activeElement) &&
        activeElement.closest(
          "[data-slot='popover-content'], [data-slot='select-content'], [data-slot='dialog-content']",
        )
      ) {
        return;
      }

      const focusableElements = Array.from(
        panel.querySelectorAll<HTMLElement>(
          "a[href], button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex='-1'])",
        ),
      ).filter((element) => element.offsetParent !== null);

      if (focusableElements.length === 0) {
        event.preventDefault();
        panel.focus();
        return;
      }

      const firstElement = focusableElements[0];
      const lastElement = focusableElements[focusableElements.length - 1];

      if (!activeElement || !panel.contains(activeElement)) {
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
    [],
  );

  const getSearchResultButtons = React.useCallback(() => {
    const resultRoot = searchResultsRef.current;
    if (!resultRoot) {
      return [];
    }

    return Array.from(
      resultRoot.querySelectorAll<HTMLButtonElement>(
        "[data-global-search-result='true']:not(:disabled)",
      ),
    );
  }, []);

  const focusSearchResult = React.useCallback(
    (position: "first" | "last") => {
      const buttons = getSearchResultButtons();
      if (buttons.length === 0) {
        return false;
      }

      buttons[position === "first" ? 0 : buttons.length - 1]?.focus();
      return true;
    },
    [getSearchResultButtons],
  );

  const focusRelativeSearchResult = React.useCallback(
    (currentButton: HTMLButtonElement, delta: 1 | -1) => {
      const buttons = getSearchResultButtons();
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
    [getSearchResultButtons],
  );

  const handleDesktopSearchKeyDown = React.useCallback(
    (event: React.KeyboardEvent<HTMLInputElement>) => {
      if (event.key === "Escape") {
        handleSearchEscape(event);
        return;
      }

      if (event.key === "Enter") {
        if (event.nativeEvent.isComposing) {
          return;
        }
        event.preventDefault();
        if (focusSearchResult("first")) {
          return;
        }
        void forceSearchGlobal();
        return;
      }

      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        const didFocus = focusSearchResult(
          event.key === "ArrowDown" ? "first" : "last",
        );
        if (didFocus) {
          event.preventDefault();
        }
      }
    },
    [focusSearchResult, forceSearchGlobal, handleSearchEscape],
  );

  const handleSearchResultKeyDown = React.useCallback(
    (event: React.KeyboardEvent<HTMLButtonElement>) => {
      if (event.key === "Escape") {
        event.preventDefault();
        handleCloseGlobalSearchPanel();
        return;
      }

      if (event.key === "Home" || event.key === "End") {
        const didFocus = focusSearchResult(
          event.key === "Home" ? "first" : "last",
        );
        if (didFocus) {
          event.preventDefault();
        }
        return;
      }

      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        const didFocus = focusRelativeSearchResult(
          event.currentTarget,
          event.key === "ArrowDown" ? 1 : -1,
        );
        if (didFocus) {
          event.preventDefault();
        }
      }
    },
    [
      focusRelativeSearchResult,
      focusSearchResult,
      handleCloseGlobalSearchPanel,
    ],
  );

  const focusDesktopSearchTab = React.useCallback((nextTab: SearchTabKey) => {
    setDesktopSearchTab(nextTab);
    const nextTabElement = desktopSearchTabRefs.current[nextTab];
    nextTabElement?.focus();
    nextTabElement?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, []);

  const handleDesktopSearchTabKeyDown = React.useCallback(
    (
      event: React.KeyboardEvent<HTMLButtonElement>,
      currentTab: SearchTabKey,
    ) => {
      const tabKeys = desktopSearchTabs.map((tab) => tab.key);
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
      focusDesktopSearchTab(nextTab);
    },
    [desktopSearchTabs, focusDesktopSearchTab],
  );

  React.useEffect(() => {
    if (!desktopSearchTabs.some((tab) => tab.key === desktopSearchTab)) {
      setDesktopSearchTab("all");
    }
  }, [desktopSearchTab, desktopSearchTabs]);

  React.useEffect(() => {
    if (!isGlobalSearchPanelOpen) {
      setDesktopSearchTab("all");
    }
  }, [isGlobalSearchPanelOpen]);

  React.useEffect(() => {
    if (isGlobalSearchPanelOpen && accountMenuOpen) {
      setAccountMenuOpen(false);
    }
  }, [accountMenuOpen, isGlobalSearchPanelOpen]);

  React.useEffect(() => {
    if (!isGlobalSearchPanelOpen || isMobile) {
      return;
    }
    searchResultsRef.current?.scrollTo({ left: 0, top: 0 });
  }, [desktopSearchTab, globalSearch, isGlobalSearchPanelOpen, isMobile]);

  React.useEffect(() => {
    if (!isGlobalSearchPanelOpen || isMobile) {
      return;
    }

    const frameId = window.requestAnimationFrame(() => {
      globalSearchInputRef.current?.focus();
      globalSearchInputRef.current?.select();
    });
    return () => window.cancelAnimationFrame(frameId);
  }, [globalSearchInputRef, isGlobalSearchPanelOpen, isMobile]);

  React.useEffect(() => {
    if (!isGlobalSearchPanelOpen || isMobile) {
      return;
    }

    const originalOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = originalOverflow;
    };
  }, [isGlobalSearchPanelOpen, isMobile]);

  React.useEffect(() => {
    const headerElement = headerRef.current;
    if (!headerElement) {
      return;
    }

    const updateHeaderHeight = () => {
      const height = headerElement.getBoundingClientRect().height;
      document.documentElement.style.setProperty(
        "--root-header-height",
        `${height}px`,
      );
      setMobileHeaderHeight(isMobile ? height : 0);
    };

    updateHeaderHeight();

    if (typeof ResizeObserver === "undefined") {
      window.addEventListener("resize", updateHeaderHeight);
      return () => window.removeEventListener("resize", updateHeaderHeight);
    }

    const resizeObserver = new ResizeObserver(() => updateHeaderHeight());
    resizeObserver.observe(headerElement);
    return () => {
      resizeObserver.disconnect();
      document.documentElement.style.removeProperty("--root-header-height");
    };
  }, [isMobile]);

  React.useEffect(() => {
    if (!isMobile || accountMenuOpen || isGlobalSearchPanelOpen) {
      setIsMobileHeaderVisible(true);
    }
  }, [accountMenuOpen, isGlobalSearchPanelOpen, isMobile]);

  React.useEffect(() => {
    if (!isMobile) {
      return;
    }

    lastScrollYRef.current = readPageScrollTop();
    let frameId: number | null = null;

    const updateHeaderVisibility = () => {
      frameId = null;
      const nextScrollY = readPageScrollTop();
      const previousScrollY = lastScrollYRef.current;
      const delta = nextScrollY - previousScrollY;

      if (Math.abs(delta) < 12) {
        return;
      }

      lastScrollYRef.current = nextScrollY;

      if (nextScrollY <= Math.max(mobileHeaderHeight, 24)) {
        setIsMobileHeaderVisible(true);
        return;
      }

      if (accountMenuOpen || isGlobalSearchPanelOpen) {
        setIsMobileHeaderVisible(true);
        return;
      }

      setIsMobileHeaderVisible(delta < 0);
    };

    const handleScroll = () => {
      if (frameId !== null) {
        return;
      }
      frameId = window.requestAnimationFrame(updateHeaderVisibility);
    };

    const scrollTargets = new Set<EventTarget>([window, document]);
    if (document.scrollingElement) {
      scrollTargets.add(document.scrollingElement);
    }
    for (const target of scrollTargets) {
      target.addEventListener("scroll", handleScroll, { passive: true });
    }

    return () => {
      if (frameId !== null) {
        window.cancelAnimationFrame(frameId);
      }
      for (const target of scrollTargets) {
        target.removeEventListener("scroll", handleScroll);
      }
    };
  }, [
    accountMenuOpen,
    isGlobalSearchPanelOpen,
    isMobile,
    mobileHeaderHeight,
    readPageScrollTop,
  ]);

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
          data-global-search-result="true"
          className="group relative flex min-w-0 items-center gap-3 overflow-hidden rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-surfA)] p-3 text-left transition hover:border-[var(--scry-bhover)] hover:bg-[rgba(12,18,36,0.70)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/25"
          onClick={() => handleRouteCommandSelect(item)}
          onKeyDown={handleSearchResultKeyDown}
          aria-label={commandLabel}
          title={commandLabel}
        >
          <span
            aria-hidden="true"
            className="absolute inset-y-3 left-0 w-0.5 rounded-r-full bg-[var(--scry-accent-ring)] opacity-0 transition group-hover:opacity-100 group-focus-visible:opacity-100"
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
          <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-[8px] text-[var(--scry-faint2)] transition group-hover:bg-[var(--scry-chip)] group-hover:text-[var(--scry-ink2)]">
            <ArrowRight className="h-4 w-4" />
          </span>
        </button>
      );
    },
    [handleRouteCommandSelect, handleSearchResultKeyDown],
  );

  const renderCatalogSection = React.useCallback(
    (items: import("@/lib/types").TitleRecord[], facet: Facet) => {
      return items.map((title) => {
        const targetView: ViewId = viewFromFacet(facet);
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
            id={selectorId("global-search-catalog-result", facet, title.id)}
            key={title.id}
            type="button"
            onClick={() => {
              resetGlobalSearch();
              globalSearchInputRef.current?.blur();
              onOpenOverview?.(targetView, {
                id: title.id,
                slug: title.slug ?? null,
                libraryId: title.libraryId,
                librarySlug: title.librarySlug ?? null,
              });
            }}
            data-global-search-result="true"
            onKeyDown={handleSearchResultKeyDown}
            className="group flex w-full items-center gap-[13px] rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-surfA)] p-2.5 text-left transition hover:border-[var(--scry-bhover)] hover:bg-[rgba(12,18,36,0.70)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/25"
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
              <p className="truncate text-sm font-semibold text-[var(--scry-ink2)]">
                {title.name}
              </p>
              <p className="mt-0.5 truncate text-[11.5px] text-[var(--scry-muted)]">
                {title.monitored
                  ? t("search.monitored")
                  : t("search.unmonitored")}
                {secondaryParts.length > 0 ? (
                  <> · {secondaryParts.join(" · ")}</>
                ) : null}
              </p>
              <div className="mt-2 flex min-w-0 flex-wrap items-center gap-2">
                <span className="max-w-[9rem] truncate rounded-md bg-[rgba(var(--scry-accent-rgb),0.16)] px-2 py-0.5 text-[10px] font-bold uppercase tracking-wide text-[var(--scry-accent-text)]">
                  {libraryLabel}
                </span>
                <span className="inline-flex min-w-0 max-w-[9rem] items-center gap-1 text-[11px] font-medium text-[#4ade80]">
                  <CircleCheck className="h-3 w-3 shrink-0" />
                  <span className="truncate">
                    {qualityLabel ?? t("search.inLibrary")}
                  </span>
                </span>
              </div>
            </div>
            <span className="hidden h-[34px] shrink-0 items-center gap-1.5 rounded-[9px] border border-[var(--scry-bhover2)] bg-[var(--scry-soft3)] px-3.5 text-[12.5px] font-semibold text-[var(--scry-body)] transition group-hover:border-primary/40 group-hover:text-[var(--scry-ink2)] sm:inline-flex">
              <Eye className="h-3.5 w-3.5" />
              {t("search.view")}
            </span>
          </button>
        );
      });
    },
    [
      globalSearchInputRef,
      handleSearchResultKeyDown,
      onOpenOverview,
      resetGlobalSearch,
      t,
    ],
  );

  const handleSearchPanelBackdropMouseDown = handleCloseGlobalSearchPanel;

  React.useEffect(() => {
    if (!isGlobalSearchPanelOpen || isMobile) {
      return;
    }

    const handleGlobalSearchPanelPointerDown = (event: PointerEvent) => {
      const target = event.target as Node | null;
      const targetElement = target instanceof Element ? target : null;
      if (target && searchShellRef.current?.contains(target)) {
        return;
      }
      if (target && searchPanelRef.current?.contains(target)) {
        return;
      }
      if (
        targetElement?.closest(
          "[data-slot='select-content'], [data-slot='popover-content']",
        )
      ) {
        return;
      }
      if (
        targetElement?.closest(
          "[data-slot='dialog-overlay'], [data-slot='dialog-content']",
        )
      ) {
        return;
      }
      handleCloseGlobalSearchPanel();
    };

    window.addEventListener("pointerdown", handleGlobalSearchPanelPointerDown);
    return () =>
      window.removeEventListener(
        "pointerdown",
        handleGlobalSearchPanelPointerDown,
      );
  }, [handleCloseGlobalSearchPanel, isMobile, isGlobalSearchPanelOpen]);

  React.useEffect(() => {
    if (!isGlobalSearchPanelOpen || isMobile) {
      return;
    }

    const handleGlobalSearchPanelEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") {
        return;
      }
      if (addDialogTarget !== null || requestDialogTarget !== null) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      handleCloseGlobalSearchPanel();
    };

    window.addEventListener("keydown", handleGlobalSearchPanelEscape);
    return () =>
      window.removeEventListener("keydown", handleGlobalSearchPanelEscape);
  }, [
    addDialogTarget,
    handleCloseGlobalSearchPanel,
    isMobile,
    isGlobalSearchPanelOpen,
    requestDialogTarget,
  ]);

  const renderMetadataSection = React.useCallback(
    (items: MetadataTvdbSearchItem[], facet: Facet, _section: string) => {
      return items.map((result) => {
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
            handleOpenRequestDialog(result, facet);
            return;
          }

          handleOpenAddDialog(result, facet);
        };
        return (
          <button
            id={globalSearchMetadataResultId(facet, result)}
            key={`${facet}-${result.tvdbId}-${result.name}`}
            type="button"
            className="group w-[124px] flex-none rounded-[12px] text-left outline-none transition focus-visible:ring-2 focus-visible:ring-primary/35 disabled:cursor-default disabled:opacity-75"
            data-global-search-result="true"
            onClick={handleMetadataAction}
            onKeyDown={handleSearchResultKeyDown}
            disabled={disabled}
            aria-label={actionTitle}
            title={actionTitle}
          >
            <div className="relative mb-2 h-[184px] overflow-hidden rounded-[11px] border border-[var(--scry-border2)] bg-[var(--scry-surfA)] shadow-[0_8px_20px_rgba(0,0,0,0.40)] transition group-hover:border-[var(--scry-bhover2)] group-hover:shadow-[0_12px_26px_rgba(0,0,0,0.50)]">
              <TitlePosterSlot
                src={posterUrl}
                alt={t("media.posterAlt", { name: result.name })}
                className="h-full w-full object-cover"
                placeholderClassName="flex h-full w-full items-center justify-center text-xs text-muted-foreground"
                emptyLabel={t("label.noArt")}
                loading="lazy"
              />
              <div className="pointer-events-none absolute inset-0 bg-[linear-gradient(180deg,transparent_50%,rgba(4,6,12,0.90))]" />
              <p className="pointer-events-none absolute inset-x-2.5 bottom-2 line-clamp-2 text-[13px] font-bold leading-[1.06] text-white shadow-black drop-shadow">
                {result.name}
              </p>
              <span
                id={actionId}
                className={cn(
                  "absolute right-2 top-2 inline-flex h-8 w-8 items-center justify-center rounded-lg shadow-lg transition",
                  disabled
                    ? "bg-[var(--scry-card2)] text-[var(--scry-muted2)]"
                    : "bg-primary text-primary-foreground hover:bg-primary/90",
                )}
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
            <div className="min-w-0 px-0.5">
              <div className="flex items-center justify-between gap-2">
                <span className="truncate text-[11px] text-[var(--scry-muted)]">
                  {result.year ? result.year : t("label.yearUnknown")}
                </span>
                <span
                  className={cn(
                    "inline-flex min-w-0 items-center gap-1 rounded-md text-[11px] font-semibold transition",
                    disabled
                      ? "text-[var(--scry-muted3)]"
                      : "text-[var(--scry-accent-text)] group-hover:text-primary",
                  )}
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
      });
    },
    [
      handleOpenAddDialog,
      handleOpenRequestDialog,
      handleSearchResultKeyDown,
      isMetadataSearchResultInCatalog,
      catalogQualityProfileOptions.length,
      librariesByFacet,
      requestableLibrariesByFacet,
      t,
    ],
  );
  const accountInitial = (user?.username.trim().charAt(0) || "?").toUpperCase();
  const isOperatorAccount = user
    ? hasAnyAppPermission(user, [
        APP_PERMISSIONS.manageUsers,
        APP_PERMISSIONS.managePermissions,
        APP_PERMISSIONS.manageSystemSettings,
        APP_PERMISSIONS.manageCatalogSettings,
      ]) ||
      hasAnyLibraryPermission(user, LIBRARY_PERMISSIONS.manageTitles) ||
      hasAnyLibraryPermission(user, LIBRARY_PERMISSIONS.manageLibrary) ||
      hasAnyLibraryPermission(user, LIBRARY_PERMISSIONS.resolveImports)
    : false;
  const accountRoleLabel = isOperatorAccount
    ? t("profile.operator")
    : t("profile.member");
  const accountKindLabel =
    user?.accountKind === "external_auto_provisioned"
      ? t("profile.externalAccount")
      : t("profile.localAccount");

  return (
    <>
      <header
        ref={headerRef}
        data-slot="root-header"
        className={cn(
          "relative z-50 border-b border-[var(--scry-border3)] bg-[var(--scry-bg)] pt-safe-comfort px-safe backdrop-blur transition-transform duration-200 ease-out",
          isMobile
            ? "fixed inset-x-0 top-[var(--root-shell-top-offset,0px)]"
            : "sticky top-0",
          isMobile
            ? !isMobileHeaderVisible
              ? "-translate-y-full"
              : "translate-y-0"
            : null,
        )}
      >
        <div className="flex w-full items-center gap-3 px-[15px] py-[11px] sm:gap-[18px] sm:px-[26px] sm:py-3.5">
          {mobileNavigation ? (
            <div className="shrink-0 min-[981px]:hidden">
              {mobileNavigation}
            </div>
          ) : null}
          <div className="relative flex min-w-0 flex-1 items-center gap-3">
            <div ref={searchShellRef} className="relative w-full max-w-[560px]">
              {isMobile ? (
                <button
                  id="global-search-mobile-trigger"
                  ref={searchTriggerRef}
                  type="button"
                  className="relative flex h-10 w-full items-center gap-3 rounded-[11px] border border-[var(--scry-border2)] bg-[var(--scry-bg)] py-0 pl-10 pr-3 text-left text-[13.5px] text-[var(--scry-faint)] shadow-none transition active:bg-accent/70 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/25"
                  onClick={() => openGlobalSearchPanel(true)}
                  aria-label={t("search.title")}
                  aria-haspopup="dialog"
                  aria-expanded={isGlobalSearchPanelOpen}
                  aria-controls="mobile-global-search-panel"
                  aria-keyshortcuts="Meta+K Control+K /"
                >
                  <Search className="absolute left-3.5 top-1/2 h-[17px] w-[17px] -translate-y-1/2 shrink-0 text-[var(--scry-faint2)]" />
                  <span className="min-w-0 flex-1 truncate">
                    {globalSearch.trim() || searchTriggerPlaceholder}
                  </span>
                </button>
              ) : (
                <button
                  id="global-search-trigger"
                  ref={searchTriggerRef}
                  type="button"
                  className={cn(
                    "relative flex h-10 w-full items-center gap-3 rounded-[11px] border border-[var(--scry-border2)] bg-[var(--scry-bg)] py-0 pl-10 pr-3 text-left text-[13.5px] text-[var(--scry-faint)] shadow-none transition hover:border-[var(--scry-bhover2)] hover:text-[var(--scry-muted)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/35",
                    isGlobalSearchPanelOpen
                      ? "border-[var(--scry-baccent)] bg-[var(--scry-hover)] text-[var(--scry-muted)] shadow-[0_0_0_1px_rgba(var(--scry-accent-rgb),0.10),0_10px_24px_rgba(0,0,0,0.18)]"
                      : null,
                  )}
                  onClick={() => openGlobalSearchPanel(true)}
                  aria-label={t("search.title")}
                  aria-haspopup="dialog"
                  aria-expanded={isGlobalSearchPanelOpen}
                  aria-controls="global-search-panel"
                  aria-keyshortcuts="Meta+K Control+K /"
                >
                  <Search className="absolute left-3.5 top-1/2 h-[17px] w-[17px] -translate-y-1/2 shrink-0 text-[var(--scry-faint2)]" />
                  <span className="min-w-0 flex-1 truncate">
                    {globalSearch.trim() || searchTriggerPlaceholder}
                  </span>
                  <kbd className="rounded-md border border-[var(--scry-kbdbd)] bg-[var(--scry-kbdbg)] px-1.5 py-0.5 text-[10px] font-medium text-[var(--scry-muted)]">
                    {searchShortcutHint}
                  </kbd>
                </button>
              )}
              {isGlobalSearchPanelOpen &&
              !isMobile &&
              typeof document !== "undefined"
                ? createPortal(
                    <div
                      data-slot="global-search-overlay"
                      className="fixed inset-0 z-[60] flex flex-col items-center bg-[rgba(2,4,10,0.66)] px-5 pb-7 pt-[max(4rem,calc(var(--root-header-height,0px)+1rem))] backdrop-blur-md motion-safe:animate-in motion-safe:fade-in-0"
                      onMouseDown={handleSearchPanelBackdropMouseDown}
                    >
                      <div
                        ref={searchPanelRef}
                        id="global-search-panel"
                        data-slot="global-search-panel"
                        role="dialog"
                        aria-modal="true"
                        aria-label={t("search.title")}
                        aria-describedby="global-search-description"
                        tabIndex={-1}
                        className="flex max-h-[calc(100dvh-7rem)] w-[min(920px,calc(100vw-2.5rem))] flex-col overflow-hidden rounded-[18px] border border-[#1f2c46] bg-[linear-gradient(180deg,var(--scry-soft),#080d1a)] shadow-[0_40px_120px_rgba(0,0,0,0.7)] outline-none motion-safe:animate-in motion-safe:slide-in-from-top-3"
                        onMouseDown={(event) => event.stopPropagation()}
                        onKeyDown={handleSearchPanelKeyDown}
                      >
                        <div className="flex items-center gap-[13px] border-b border-[var(--scry-border)] px-[18px] py-4">
                          <Search className="h-[21px] w-[21px] shrink-0 text-[var(--scry-accent-ring)]" />
                          <div className="min-w-0 flex-1">
                            <Input
                              id="global-search-input"
                              ref={globalSearchInputRef}
                              autoFocus
                              value={globalSearch}
                              onChange={handleSearchChange}
                              onKeyDown={handleDesktopSearchKeyDown}
                              data-ui="global-search"
                              className="h-8 border-0 bg-transparent px-0 text-[17px] text-[#f4f7fb] shadow-none placeholder:text-[17px] placeholder:text-[var(--scry-muted3)] focus-visible:ring-0"
                              placeholder={searchOverlayPlaceholder}
                              aria-label={searchOverlayPlaceholder}
                              aria-controls="global-search-results-panel"
                              aria-describedby="global-search-description global-search-status"
                            />
                            <p
                              id="global-search-description"
                              className="truncate text-xs text-[var(--scry-muted3)]"
                            >
                              {globalSearch.trim()
                                ? t("search.subtitleWithQuery", {
                                    query: globalSearch.trim(),
                                  })
                                : searchSubtitle}
                            </p>
                            <p
                              id="global-search-status"
                              className="sr-only"
                              role="status"
                              aria-live="polite"
                              aria-atomic="true"
                            >
                              {searchStatusLabel}
                            </p>
                          </div>
                          {globalSearch ? (
                            <button
                              id="global-search-clear"
                              type="button"
                              className="inline-flex h-[26px] w-[26px] shrink-0 items-center justify-center rounded-[7px] bg-[var(--scry-kbdbg)] text-[var(--scry-muted2)] transition hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)]"
                              onClick={handleClearSearch}
                              aria-label={t("label.clear")}
                            >
                              <Eraser className="h-3.5 w-3.5" />
                            </button>
                          ) : null}
                          <kbd className="rounded-[7px] border border-[var(--scry-kbdbd)] bg-transparent px-2 py-1 text-[11px] font-medium text-[var(--scry-faint2)]">
                            ESC
                          </kbd>
                          <button
                            type="button"
                            className="inline-flex h-[30px] w-[30px] shrink-0 items-center justify-center rounded-lg bg-[var(--scry-kbdbg)] text-[var(--scry-muted2)] transition hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)]"
                            onClick={handleCloseGlobalSearchPanel}
                            aria-label={t("label.close")}
                          >
                            <X className="h-4 w-4" />
                          </button>
                        </div>
                        <div
                          className="flex gap-2 overflow-x-auto border-b border-[var(--scry-border)] px-[18px] py-[13px] [-ms-overflow-style:none] [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
                          role="tablist"
                          aria-label={t("search.title")}
                        >
                          {desktopSearchTabs.map((tab) => (
                            <button
                              id={`global-search-tab-${tab.key}`}
                              key={tab.key}
                              ref={(node) => {
                                desktopSearchTabRefs.current[tab.key] = node;
                              }}
                              type="button"
                              role="tab"
                              aria-selected={desktopSearchTab === tab.key}
                              aria-controls="global-search-results-panel"
                              tabIndex={desktopSearchTab === tab.key ? 0 : -1}
                              className={cn(
                                "inline-flex h-8 shrink-0 items-center gap-1.5 rounded-[9px] border px-[13px] text-[12.5px] font-semibold transition focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/35",
                                desktopSearchTab === tab.key
                                  ? "border-transparent bg-[var(--scry-accent-grad)] text-primary-foreground shadow-[0_8px_18px_rgba(var(--scry-accent-rgb),0.26)]"
                                  : "border-[var(--scry-border2)] bg-[var(--scry-card2)] text-[var(--scry-muted2)] hover:border-[var(--scry-bhover2)] hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)]",
                              )}
                              onMouseDown={(event) => event.preventDefault()}
                              onClick={() => setDesktopSearchTab(tab.key)}
                              onKeyDown={(event) =>
                                handleDesktopSearchTabKeyDown(event, tab.key)
                              }
                            >
                              {tab.label}
                              <span
                                className={cn(
                                  "font-medium tabular-nums",
                                  desktopSearchTab === tab.key
                                    ? "text-primary-foreground/80"
                                    : "text-[var(--scry-muted2)]",
                                )}
                              >
                                {tab.count}
                              </span>
                            </button>
                          ))}
                        </div>
                        <div
                          ref={searchResultsRef}
                          id="global-search-results-panel"
                          data-slot="global-search-results"
                          role="tabpanel"
                          aria-labelledby={`global-search-tab-${desktopSearchTab}`}
                          className="min-h-0 flex-1 overflow-y-auto p-[18px] [scrollbar-color:var(--scry-border2)_transparent] [scrollbar-width:thin] [&::-webkit-scrollbar]:w-2.5 [&::-webkit-scrollbar-track]:bg-transparent [&::-webkit-scrollbar-thumb]:rounded-full [&::-webkit-scrollbar-thumb]:border-[3px] [&::-webkit-scrollbar-thumb]:border-transparent [&::-webkit-scrollbar-thumb]:bg-[var(--scry-border2)] [&::-webkit-scrollbar-thumb]:bg-clip-content"
                        >
                          {showSectionResults ? (
                            <div className="space-y-6">
                              {showCatalogSection ? (
                                <section
                                  id="global-search-catalog-section"
                                  className="space-y-3"
                                >
                                  <div className="flex items-baseline justify-between gap-3">
                                    <div className="flex items-baseline gap-2">
                                      <h3 className="text-[15px] font-bold text-[var(--scry-ink2)]">
                                        {t("search.inLibrary")}
                                      </h3>
                                      <span className="text-xs text-[var(--scry-muted3)]">
                                        {t("search.alreadyInCollection")}
                                      </span>
                                    </div>
                                    <div className="flex shrink-0 items-center gap-3">
                                      {!catalogSearchLoading &&
                                      desktopSearchTab !== "library" &&
                                      hiddenCatalogResultCount > 0 ? (
                                        <button
                                          type="button"
                                          className="text-xs font-medium text-[var(--scry-accent-ring)] transition hover:text-[var(--scry-accent-text)]"
                                          onMouseDown={(event) =>
                                            event.preventDefault()
                                          }
                                          onClick={() =>
                                            focusDesktopSearchTab("library")
                                          }
                                          aria-label={`${t("search.viewAll")} ${t("search.inLibrary")}`}
                                        >
                                          {t("search.viewAll")}
                                        </button>
                                      ) : null}
                                      <span className="text-xs font-medium tabular-nums text-[var(--scry-muted3)]">
                                        {visibleCatalogCount === 1
                                          ? t("search.resultCountOne")
                                          : t("search.resultCountOther", {
                                              count:
                                                String(visibleCatalogCount),
                                            })}
                                      </span>
                                    </div>
                                  </div>
                                  {catalogSearchLoading ? (
                                    <SearchSectionLoading
                                      label={t("label.loading")}
                                    />
                                  ) : visibleCatalogResults.length === 0 ? (
                                    <p className="rounded-[12px] border border-dashed border-[var(--scry-border2)] bg-[var(--scry-surfC)] px-4 py-5 text-sm text-[var(--scry-muted3)]">
                                      {!hasMinimumGlobalSearchQuery
                                        ? searchMinimumQueryHint
                                        : t("search.noCatalogMatches")}
                                    </p>
                                  ) : (
                                    <div className="flex flex-col gap-3">
                                      {visibleCatalogResults.flatMap(
                                        ({ facet, title }) =>
                                          renderCatalogSection([title], facet),
                                      )}
                                    </div>
                                  )}
                                </section>
                              ) : null}
                              {metadataSectionFacets.length > 0 ? (
                                <div className="space-y-5">
                                  {metadataSectionFacets.map((f) => {
                                    const items =
                                      metadataSearchResults[f.metadataKey] ??
                                      [];
                                    const visibleItems =
                                      desktopSearchTab === "all"
                                        ? items.slice(0, 6)
                                        : items;
                                    const hiddenItemCount = Math.max(
                                      items.length - visibleItems.length,
                                      0,
                                    );
                                    const facetLabel = t(f.navLabelKey);
                                    const viewAllFacetLabel =
                                      viewAllLabelForFacet(t, f.id);
                                    const resultCountLabel =
                                      items.length === 1
                                        ? t("search.resultCountOne")
                                        : t("search.resultCountOther", {
                                            count: String(items.length),
                                          });
                                    return (
                                      <section
                                        key={f.id}
                                        id={selectorId(
                                          "global-search-metadata-section",
                                          f.id,
                                        )}
                                        className="space-y-3"
                                      >
                                        <div className="flex items-baseline justify-between gap-3">
                                          <div className="flex min-w-0 items-baseline gap-2">
                                            <h3 className="truncate text-[15px] font-bold text-[var(--scry-ink2)]">
                                              {facetLabel}
                                            </h3>
                                            <span className="shrink-0 text-xs text-[var(--scry-muted3)]">
                                              {metadataSearchLoading
                                                ? t("search.metadataSearch")
                                                : resultCountLabel}
                                            </span>
                                          </div>
                                          <div className="flex shrink-0 items-center gap-3">
                                            {desktopSearchTab === "all" &&
                                            hiddenItemCount > 0 ? (
                                              <button
                                                type="button"
                                                className="text-xs font-medium normal-case text-[var(--scry-accent-ring)] transition hover:text-[var(--scry-accent-text)]"
                                                onMouseDown={(event) =>
                                                  event.preventDefault()
                                                }
                                                onClick={() =>
                                                  focusDesktopSearchTab(f.id)
                                                }
                                                aria-label={viewAllFacetLabel}
                                              >
                                                {viewAllFacetLabel}
                                              </button>
                                            ) : null}
                                          </div>
                                        </div>
                                        {metadataSearchLoading ? (
                                          <SearchSectionLoading
                                            label={t("label.loading")}
                                          />
                                        ) : items.length === 0 ? (
                                          <p className="rounded-[12px] border border-dashed border-[var(--scry-border2)] bg-[var(--scry-surfC)] px-4 py-5 text-sm text-[var(--scry-muted3)]">
                                            {t("search.noMetadataMatches")}
                                          </p>
                                        ) : (
                                          <div className="flex gap-3 overflow-x-auto pb-1 [-ms-overflow-style:none] [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
                                            {renderMetadataSection(
                                              visibleItems,
                                              f.id,
                                              f.metadataKey,
                                            )}
                                          </div>
                                        )}
                                      </section>
                                    );
                                  })}
                                </div>
                              ) : null}
                              {showRouteCommandSection ? (
                                <section className="space-y-3">
                                  <div className="flex items-baseline justify-between gap-3">
                                    <div className="flex min-w-0 items-baseline gap-2">
                                      <h3 className="truncate text-[15px] font-bold text-[var(--scry-ink2)]">
                                        {t("search.actionsAndSettings")}
                                      </h3>
                                      <span className="shrink-0 text-xs text-[var(--scry-muted3)]">
                                        {routeCommandResults.length === 1
                                          ? t("search.resultCountOne")
                                          : t("search.resultCountOther", {
                                              count: String(
                                                routeCommandResults.length,
                                              ),
                                            })}
                                      </span>
                                    </div>
                                    {desktopSearchTab === "all" &&
                                    hiddenRouteCommandResultCount > 0 ? (
                                      <button
                                        type="button"
                                        className="text-xs font-medium normal-case text-[var(--scry-accent-ring)] transition hover:text-[var(--scry-accent-text)]"
                                        onMouseDown={(event) =>
                                          event.preventDefault()
                                        }
                                        onClick={() =>
                                          focusDesktopSearchTab("actions")
                                        }
                                      >
                                        {t("search.viewAll")}
                                      </button>
                                    ) : null}
                                  </div>
                                  <div className="space-y-3">
                                    {groupRouteCommandItems(
                                      visibleRouteCommandResults,
                                    ).map((group) => (
                                      <div
                                        key={group.groupLabel ?? "ungrouped"}
                                        className="space-y-2"
                                      >
                                        {group.groupLabel ? (
                                          <p className="text-[11px] font-semibold uppercase tracking-wide text-[var(--scry-muted3)]">
                                            {group.groupLabel}
                                          </p>
                                        ) : null}
                                        <div className="grid gap-2 sm:grid-cols-2">
                                          {group.items.map(
                                            renderRouteCommandItem,
                                          )}
                                        </div>
                                      </div>
                                    ))}
                                  </div>
                                </section>
                              ) : null}
                              <div className="flex flex-wrap items-center justify-center gap-2 pt-1 text-xs text-[var(--scry-muted3)]">
                                <Info className="h-3.5 w-3.5 text-[var(--scry-faint2)]" />
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
                                    className="z-[70] w-72 border-[var(--scry-border2)] bg-[linear-gradient(180deg,var(--scry-soft),var(--scry-bg))] p-3 text-xs shadow-[0_18px_48px_rgba(0,0,0,0.32)]"
                                  >
                                    <div className="space-y-2 text-[var(--scry-muted3)]">
                                      <p>{searchTipTitles}</p>
                                      <p>{searchTipTabs}</p>
                                      {canViewCatalog ? (
                                        <p>{t("search.tipIndexers")}</p>
                                      ) : null}
                                    </div>
                                  </PopoverContent>
                                </Popover>
                              </div>
                            </div>
                          ) : searching ? (
                            <div className="py-12">
                              <SearchSectionLoading
                                label={t("label.searching")}
                              />
                            </div>
                          ) : (
                            <div className="flex flex-col items-center justify-center py-14 text-center">
                              <div className="mb-4 flex h-14 w-14 items-center justify-center rounded-[15px] border border-[var(--scry-border2)] bg-[var(--scry-chip)] text-[var(--scry-faint2)]">
                                {hasMinimumGlobalSearchQuery ? (
                                  <SearchX className="h-6 w-6" />
                                ) : (
                                  <Search className="h-6 w-6" />
                                )}
                              </div>
                              <p className="text-[17px] font-bold text-[var(--scry-ink2)]">
                                {hasMinimumGlobalSearchQuery
                                  ? t("search.noMatchesFor", {
                                      query: trimmedGlobalSearch,
                                    })
                                  : trimmedGlobalSearch
                                    ? t("search.minimumQueryTitle")
                                    : searchOverlayPlaceholder}
                              </p>
                              <p className="mt-1 max-w-sm text-sm leading-6 text-[var(--scry-muted3)]">
                                {trimmedGlobalSearch &&
                                !hasMinimumGlobalSearchQuery
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
                                  className="z-[70] w-72 border-[var(--scry-border2)] bg-[linear-gradient(180deg,var(--scry-soft),var(--scry-bg))] p-3 text-xs shadow-[0_18px_48px_rgba(0,0,0,0.32)]"
                                >
                                  <div className="space-y-2 text-[var(--scry-muted3)]">
                                    <p>{searchTipTitles}</p>
                                    <p>{searchTipTabs}</p>
                                    {canViewCatalog ? (
                                      <p>{t("search.tipIndexers")}</p>
                                    ) : null}
                                  </div>
                                </PopoverContent>
                              </Popover>
                            </div>
                          )}
                        </div>
                      </div>
                    </div>,
                    document.body,
                  )
                : null}
            </div>
          </div>
          {user ? (
            <Popover open={accountMenuOpen} onOpenChange={setAccountMenuOpen}>
              <PopoverTrigger asChild>
                <Button
                  id="account-menu-trigger"
                  type="button"
                  variant="ghost"
                  className={cn(
                    "h-10 shrink-0 gap-2 rounded-[11px] border border-[var(--scry-border2)] bg-[var(--scry-bg)] p-[5px] pr-2 text-[var(--scry-body)] shadow-none transition hover:border-[var(--scry-bhover2)] hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)] sm:h-10",
                    accountMenuOpen
                      ? "border-[var(--scry-baccent)] bg-[var(--scry-hover)] text-[var(--scry-ink2)]"
                      : null,
                  )}
                  aria-label={t("profile.accountInfo")}
                  aria-haspopup="dialog"
                  aria-controls="account-menu-content"
                  aria-expanded={accountMenuOpen}
                >
                  <span className="flex h-[30px] w-[30px] items-center justify-center rounded-lg bg-[var(--scry-accent-grad)] text-[13px] font-bold text-primary-foreground shadow-[0_8px_18px_rgba(var(--scry-accent-rgb),0.25)]">
                    {accountInitial}
                  </span>
                  <span className="hidden min-w-0 flex-col items-start leading-tight sm:flex">
                    <span className="max-w-32 truncate text-[12.5px] font-semibold text-[var(--scry-ink2)]">
                      {user.username}
                    </span>
                    <span className="text-[10.5px] font-medium text-[var(--scry-faint)]">
                      {accountRoleLabel}
                    </span>
                  </span>
                  <ChevronsUpDown className="h-4 w-4 text-[var(--scry-faint)]" />
                </Button>
              </PopoverTrigger>
              <PopoverContent
                id="account-menu-content"
                align="end"
                sideOffset={8}
                className="w-[min(18rem,calc(100vw-1rem))] border-[var(--scry-border2)] bg-[linear-gradient(180deg,var(--scry-soft),var(--scry-bg))] p-2 shadow-[0_18px_48px_rgba(0,0,0,0.36)] sm:w-56"
              >
                <div className="border-b border-[var(--scry-border)] px-3 pb-2 pt-1.5">
                  <p className="truncate text-sm font-medium text-foreground">
                    {user.username}
                  </p>
                  <div className="mt-1 flex min-w-0 flex-wrap items-center gap-1.5">
                    <span className="rounded-md bg-[rgba(var(--scry-accent-rgb),0.16)] px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-[var(--scry-accent-text)]">
                      {accountRoleLabel}
                    </span>
                    <span className="rounded-md bg-[var(--scry-kbdbg)] px-1.5 py-0.5 text-[10px] font-medium text-[var(--scry-muted)]">
                      {accountKindLabel}
                    </span>
                  </div>
                </div>
                <div className="space-y-1 pt-2">
                  <Button
                    id="account-menu-profile"
                    type="button"
                    variant="ghost"
                    className="h-11 w-full justify-start rounded-[9px] px-3 text-sm font-medium text-[var(--scry-body)] transition hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)] focus-visible:ring-primary/25"
                    onClick={handleOpenProfile}
                  >
                    <User className="mr-2 h-4 w-4 text-[var(--scry-accent-ring)]" />
                    {t("settings.profile")}
                  </Button>
                  {token && effectiveFormLoginEnabled !== false ? (
                    <Button
                      id="account-menu-logout"
                      type="button"
                      variant="ghost"
                      className="h-11 w-full justify-start rounded-[9px] px-3 text-sm font-medium text-destructive transition hover:bg-destructive/10 hover:text-destructive focus-visible:ring-destructive/25"
                      onClick={handleLogout}
                    >
                      <LogOut className="mr-2 h-4 w-4" />
                      {t("auth.logoutButton")}
                    </Button>
                  ) : null}
                </div>
              </PopoverContent>
            </Popover>
          ) : null}
        </div>
      </header>
      {isMobile ? (
        <div
          aria-hidden="true"
          className="shrink-0 transition-[height] duration-200 ease-out"
          style={{ height: isMobileHeaderVisible ? mobileHeaderHeight : 0 }}
        />
      ) : null}
      {isGlobalSearchPanelOpen && isMobile ? (
        <MobileSearchOverlay
          canViewCatalog={canViewCatalog}
          onClose={handleCloseGlobalSearchPanel}
          onOpenOverview={onOpenOverview}
          routeCommandItems={routeCommandItems}
        />
      ) : null}
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
    </>
  );
});
