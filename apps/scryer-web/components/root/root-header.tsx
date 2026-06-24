
import * as React from "react";
import { ChevronDown, Loader2, LogOut, Plus, Search, Send, User, UserRound, X } from "lucide-react";
import { useNavigate } from "react-router-dom";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { RouteCommandPalette } from "@/components/common/route-command-palette";
import type { OverviewTitleTarget, ViewId } from "@/components/root/types";
import { useTranslate } from "@/lib/context/translate-context";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import type { Facet } from "@/lib/types";
import type {
  MetadataCatalogAddOptions,
  MetadataCatalogRequestOptions,
} from "@/lib/hooks/use-global-search";
import type { RouteCommandPaletteConfig } from "@/components/common/route-command-palette";
import { useAuth } from "@/lib/hooks/use-auth";
import { useIsMobile } from "@/lib/hooks/use-mobile";
import { MobileSearchOverlay } from "@/components/root/mobile-search-overlay";
import { FACET_REGISTRY } from "@/lib/facets/registry";
import {
  sectionLabelForFacet,
  viewFromFacet,
} from "@/lib/facets/helpers";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";
import { TitlePosterSlot } from "@/components/title-poster-slot";
import { useSearchContext } from "@/lib/context/search-context";
import { cn } from "@/lib/utils";
import { buildViewPath } from "@/lib/utils/routing";
import {
  globalSearchConfigureAddId,
  globalSearchMetadataResultId,
  globalSearchRequestId,
  selectorId,
} from "@/lib/utils/dom-ids";
import { AddToCatalogDialog, EMPTY_SEARCH_RESULT } from "@/components/root/add-to-catalog-dialog";
import { RequestMediaDialog } from "@/components/root/request-media-dialog";


type RootHeaderProps = {
  routeCommandPalette?: RouteCommandPaletteConfig;
  onOpenOverview?: (targetView: ViewId, overviewTarget: OverviewTitleTarget) => void;
};

function catalogFacetFromString(facet: string): Facet {
  return facet === "movie" ? "movie" : facet === "anime" ? "anime" : "series";
}

function SearchSectionLoading({ label }: { label: string }) {
  return (
    <div className="flex min-h-24 items-center gap-3 rounded-lg border border-dashed border-border/80 bg-muted/30 px-4 py-3 text-sm text-muted-foreground">
      <Loader2 className="h-4 w-4 animate-spin text-emerald-500" />
      <span>{label}</span>
    </div>
  );
}

export const RootHeader = React.memo(function RootHeader({
  routeCommandPalette,
  onOpenOverview,
}: RootHeaderProps) {
  const searchState = useSearchContext();
  const {
    resolveDefaultQualityProfileIdForFacet,
    addMetadataSearchResultToCatalog,
    requestMetadataSearchResult,
    closeGlobalSearchPanel,
    resetGlobalSearch,
    openGlobalSearchPanel,
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
  const isMobile = useIsMobile();
  const navigate = useNavigate();
  const { token, user, logout, effectiveFormLoginEnabled } = useAuth();
  const headerRef = React.useRef<HTMLElement>(null);
  const searchShellRef = React.useRef<HTMLDivElement>(null);
  const searchPanelRef = React.useRef<HTMLDivElement>(null);
  const lastScrollYRef = React.useRef(0);
  const [accountMenuOpen, setAccountMenuOpen] = React.useState(false);
  const [mobileHeaderHeight, setMobileHeaderHeight] = React.useState(0);
  const [isMobileHeaderVisible, setIsMobileHeaderVisible] = React.useState(true);
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
  const hasAnyMatches =
    catalogSearchResults.length > 0 ||
    FACET_REGISTRY.some((f) => (metadataSearchResults[f.metadataKey] ?? []).length > 0);
  const showSectionResults =
    catalogSearchLoading || metadataSearchLoading || hasAnyMatches;

  const catalogSearchSections = React.useMemo(
    () => Object.fromEntries(
      FACET_REGISTRY.map((f) => [
        f.id,
        catalogSearchResults.filter((title) => catalogFacetFromString(title.facet) === f.id),
      ]),
    ) as Record<Facet, import("@/lib/types").TitleRecord[]>,
    [catalogSearchResults],
  );
  const [addDialogTarget, setAddDialogTarget] = React.useState<{
    result: MetadataTvdbSearchItem;
    facet: Facet;
  } | null>(null);
  const [requestDialogTarget, setRequestDialogTarget] = React.useState<{
    result: MetadataTvdbSearchItem;
    facet: Facet;
  } | null>(null);
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
        globalSearchInputRef.current?.blur();
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
    async (result: MetadataTvdbSearchItem, facet: Facet, options: MetadataCatalogRequestOptions) => {
      const accepted = await requestMetadataSearchResult(result, facet, options);
      if (accepted) {
        resetGlobalSearch();
        globalSearchInputRef.current?.blur();
      }
      return accepted;
    },
    [globalSearchInputRef, requestMetadataSearchResult, resetGlobalSearch],
  );

  const handleSearchSubmit = React.useCallback(
    (event: React.FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      void forceSearchGlobal();
    },
    [forceSearchGlobal],
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
      openGlobalSearchPanel();
    },
    [openGlobalSearchPanel, setGlobalSearch],
  );

  const handleSearchFocus = React.useCallback(() => {
    openGlobalSearchPanel(isMobile || undefined);
  }, [openGlobalSearchPanel, isMobile]);

  const handleSearchEscape = React.useCallback(
    (event: React.KeyboardEvent<HTMLInputElement>) => {
      if (event.key !== "Escape") {
        return;
      }
      closeGlobalSearchPanel();
      globalSearchInputRef.current?.blur();
    },
    [globalSearchInputRef, closeGlobalSearchPanel],
  );

  const handleClearSearch = React.useCallback(() => {
    setGlobalSearch("");
    globalSearchInputRef.current?.focus();
  }, [globalSearchInputRef, setGlobalSearch]);

  React.useEffect(() => {
    const headerElement = headerRef.current;
    if (!headerElement) {
      return;
    }

    const updateHeaderHeight = () => {
      const height = headerElement.getBoundingClientRect().height;
      document.documentElement.style.setProperty("--root-header-height", `${height}px`);
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

  const renderCatalogSection = React.useCallback(
    (items: import("@/lib/types").TitleRecord[], facet: Facet) => {
      return items.map((title) => {
        const targetView: ViewId = viewFromFacet(facet);
        const tvdbId = (title.externalIds ?? [])
          .find((externalId) => externalId.source.toLowerCase() === "tvdb")
          ?.value.trim();
        const posterUrl = selectPosterVariantUrl(title.posterUrl, "w70");
        return (
          <button
            id={selectorId("global-search-catalog-result", facet, title.name)}
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
            className="block w-full rounded-lg border border-border bg-card/60 p-3 text-left hover:bg-accent/80"
            aria-label={title.name}
          >
            <div className="mb-2 flex min-h-20 items-start gap-3">
              <div className="h-20 w-14 flex-none overflow-hidden rounded-md border border-border bg-muted">
                <TitlePosterSlot
                  src={posterUrl}
                  sourceSrc={title.posterSourceUrl}
                  metadataFetchedAt={title.metadataFetchedAt}
                  createdAt={title.createdAt}
                  alt={t("media.posterAlt", { name: title.name })}
                  className="h-full w-full object-cover"
                  placeholderClassName="flex h-full w-full items-center justify-center text-xs text-muted-foreground"
                  emptyLabel={t("label.noArt")}
                  loading="lazy"
                />
              </div>
              <div className="min-w-0">
                <p className="text-sm font-medium text-foreground">{title.name}</p>
                <p className="text-xs text-muted-foreground">
                  {sectionLabelForFacet(t, facet)} • {title.monitored ? t("label.yes") : t("label.no")}
                  {tvdbId ? <> • {tvdbId}</> : null}
                </p>
              </div>
            </div>
          </button>
        );
      });
    },
    [globalSearchInputRef, onOpenOverview, resetGlobalSearch, t],
  );

  const handleSearchPanelBackdropMouseDown = React.useCallback(() => {
    closeGlobalSearchPanel();
    globalSearchInputRef.current?.blur();
  }, [globalSearchInputRef, closeGlobalSearchPanel]);

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
      if (targetElement?.closest("[data-slot='select-content']")) {
        return;
      }
      if (targetElement?.closest("[data-slot='dialog-overlay'], [data-slot='dialog-content']")) {
        return;
      }
      closeGlobalSearchPanel();
      globalSearchInputRef.current?.blur();
    };

    window.addEventListener("pointerdown", handleGlobalSearchPanelPointerDown);
    return () => window.removeEventListener("pointerdown", handleGlobalSearchPanelPointerDown);
  }, [
    closeGlobalSearchPanel,
    globalSearchInputRef,
    isMobile,
    isGlobalSearchPanelOpen,
  ]);

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
      closeGlobalSearchPanel();
      globalSearchInputRef.current?.blur();
    };

    window.addEventListener("keydown", handleGlobalSearchPanelEscape);
    return () => window.removeEventListener("keydown", handleGlobalSearchPanelEscape);
  }, [
    addDialogTarget,
    closeGlobalSearchPanel,
    globalSearchInputRef,
    isMobile,
    isGlobalSearchPanelOpen,
    requestDialogTarget,
  ]);

  const renderMetadataSection = React.useCallback(
    (items: MetadataTvdbSearchItem[], facet: Facet, _section: string) => {
      return items.map((result) => {
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
            <div className="flex items-start justify-between gap-3">
              <div className="flex min-h-20 gap-3">
                <div className="h-20 w-14 flex-none overflow-hidden rounded-md border border-border bg-muted">
                  <TitlePosterSlot
                    src={posterUrl}
                    alt={t("media.posterAlt", { name: result.name })}
                    className="h-full w-full object-cover"
                    placeholderClassName="flex h-full w-full items-center justify-center text-xs text-muted-foreground"
                    emptyLabel={t("label.noArt")}
                    loading="lazy"
                  />
                </div>
                <div className="min-w-0">
                  <p className="text-sm font-medium text-foreground">{result.name}</p>
                  <p className="text-xs text-muted-foreground">
                    {result.year ? result.year : t("label.yearUnknown")}
                  </p>
                  {result.overview ? (
                    <p className="mt-2 text-xs text-muted-foreground line-clamp-2">
                      {result.overview}
                    </p>
                  ) : null}
                </div>
              </div>
              <div className="flex items-center self-center">
                <Button
                  id={
                    opensRequestDialog
                      ? globalSearchRequestId(facet, result)
                      : globalSearchConfigureAddId(facet, result)
                  }
                  type="button"
                  variant={disabled ? "secondary" : "default"}
                  className={
                    disabled
                      ? "h-10 w-10 bg-accent text-card-foreground px-0"
                      : "h-10 w-10 bg-emerald-500 text-foreground hover:bg-emerald-600 px-0"
                  }
                  onClick={() =>
                    opensRequestDialog
                      ? setRequestDialogTarget({ result, facet })
                      : handleOpenAddDialog(result, facet)
                  }
                  disabled={disabled}
                  aria-label={actionLabel}
                  title={actionLabel}
                >
                  {opensRequestDialog ? (
                    <Send className="h-4 w-4" />
                  ) : (
                    <Plus className="h-4 w-4" />
                  )}
                </Button>
              </div>
            </div>
          </div>
        );
      });
    },
    [
      handleOpenAddDialog,
      isMetadataSearchResultInCatalog,
      librariesByFacet,
      requestableLibrariesByFacet,
      t,
    ],
  );

  return (
    <>
      <header
        ref={headerRef}
        data-slot="root-header"
        className={cn(
          "relative z-50 border-b border-border bg-background/90 pt-safe-comfort px-safe backdrop-blur transition-transform duration-200 ease-out",
          isMobile ? "fixed inset-x-0 top-0" : "sticky top-0",
          !isMobileHeaderVisible && isMobile ? "-translate-y-full" : "translate-y-0",
        )}
      >
        <RouteCommandPalette
          config={routeCommandPalette}
          onOpenOverview={onOpenOverview}
        />
        <div className="mx-auto flex w-full max-w-[1720px] items-center gap-3 px-4 py-2.5 pr-14 sm:gap-4 sm:pr-4">
          <form
            className="relative flex min-w-0 flex-1 items-center justify-center gap-3"
            onSubmit={handleSearchSubmit}
          >
            <div ref={searchShellRef} className="relative w-full max-w-[560px]">
              <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
              <Input
                id="global-search-input"
                ref={globalSearchInputRef}
                value={globalSearch}
                onChange={handleSearchChange}
                onFocus={handleSearchFocus}
                onKeyDown={handleSearchEscape}
                data-ui="global-search"
                className="h-10 w-full rounded-xl border-border/80 bg-field/80 pl-9 pr-3 text-sm shadow-none placeholder:text-sm focus-visible:border-primary/70 focus-visible:ring-primary/25"
                placeholder={t("search.globalPlaceholder")}
                aria-label={t("search.globalPlaceholder")}
              />
              {globalSearch && !isMobile ? (
                <button
                  id="global-search-clear"
                  type="button"
                  className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground transition hover:text-foreground"
                  onMouseDown={handleClearSearch}
                  aria-label={t("label.clear")}
                >
                  <X className="h-6 w-6" />
                </button>
              ) : null}
              {isGlobalSearchPanelOpen && !isMobile ? (
                <div
                  ref={searchPanelRef}
                  id="global-search-panel"
                  data-slot="global-search-panel"
                  className="absolute left-0 top-full z-30 mt-2 w-full max-h-[65vh] overflow-y-auto rounded-xl border border-border bg-card p-4 shadow-lg"
                >
                {showSectionResults ? (
                  <div className="space-y-4">
                    <section id="global-search-catalog-section" className="space-y-2">
                      <h3 className="text-sm font-semibold text-foreground">{t("search.catalog")}</h3>
                      <div className="grid gap-4 md:grid-cols-3">
                        {FACET_REGISTRY.map((f) => {
                          const items = catalogSearchSections[f.id] ?? [];
                          return (
                            <div
                              key={f.id}
                              id={selectorId("global-search-catalog-section", f.id)}
                              className="space-y-2"
                            >
                              <h4 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                                {sectionLabelForFacet(t, f.id)}
                              </h4>
                              {catalogSearchLoading ? (
                                <SearchSectionLoading label={t("label.loading")} />
                              ) : items.length === 0 ? (
                                <p className="text-sm text-muted-foreground">
                                  {t("search.noCatalogMatches")}
                                </p>
                              ) : (
                                renderCatalogSection(items, f.id)
                              )}
                            </div>
                          );
                        })}
                      </div>
                    </section>
                      <div className={`grid gap-4 md:grid-cols-${FACET_REGISTRY.length}`}>
                        {FACET_REGISTRY.map((f) => {
                          const items = metadataSearchResults[f.metadataKey] ?? [];
                          return (
                            <section
                              key={f.id}
                              id={selectorId("global-search-metadata-section", f.id)}
                              className="space-y-2"
                            >
                              <h3 className="text-sm font-semibold text-foreground">
                                {sectionLabelForFacet(t, f.id)}
                              </h3>
                              {metadataSearchLoading ? (
                                <SearchSectionLoading label={t("label.loading")} />
                              ) : items.length === 0 ? (
                                <p className="text-sm text-muted-foreground">
                                  {t("search.noMetadataMatches")}
                                </p>
                              ) : (
                                renderMetadataSection(items, f.id, f.metadataKey)
                              )}
                            </section>
                          );
                        })}
                      </div>
                    </div>
                  ) : searching ? (
                    <div className="flex items-center gap-3 py-3">
                      <Loader2 className="h-5 w-5 animate-spin text-emerald-500" />
                      <p className="text-sm text-muted-foreground">{t("label.searching")}</p>
                    </div>
                  ) : (
                    <p className="text-sm text-muted-foreground">{t("status.nothingFound")}</p>
                  )}
                </div>
              ) : null}
            </div>
          </form>
          {user ? (
            <Popover open={accountMenuOpen} onOpenChange={setAccountMenuOpen}>
              <PopoverTrigger asChild>
                <Button
                  id="account-menu-trigger"
                  type="button"
                  variant="ghost"
                  className="h-11 shrink-0 gap-1.5 rounded-lg px-2.5 text-foreground transition hover:bg-accent/80 sm:h-10"
                  aria-label={t("profile.accountInfo")}
                  aria-expanded={accountMenuOpen}
                >
                  <UserRound className="h-5 w-5" />
                  <ChevronDown
                    className={cn(
                      "h-4 w-4 text-muted-foreground transition-transform",
                      accountMenuOpen ? "rotate-180" : "",
                    )}
                  />
                </Button>
              </PopoverTrigger>
              <PopoverContent
                align="end"
                sideOffset={8}
                className="w-[min(18rem,calc(100vw-1rem))] p-2 sm:w-56"
              >
                <div className="border-b border-border/70 px-3 pb-2 pt-1.5">
                  <p className="truncate text-sm font-medium text-foreground">{user.username}</p>
                </div>
                <div className="space-y-1 pt-2">
                  <Button
                    id="account-menu-profile"
                    type="button"
                    variant="ghost"
                    className="h-11 w-full justify-start px-3 text-sm"
                    onClick={handleOpenProfile}
                  >
                    <User className="mr-2 h-4 w-4" />
                    {t("settings.profile")}
                  </Button>
                  {token && effectiveFormLoginEnabled !== false ? (
                    <Button
                      id="account-menu-logout"
                      type="button"
                      variant="ghost"
                      className="h-11 w-full justify-start px-3 text-sm text-destructive hover:text-destructive"
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
      {isGlobalSearchPanelOpen && !isMobile ? (
        <div
          className="fixed inset-0 z-40 bg-background/80 backdrop-blur-sm"
          onMouseDown={handleSearchPanelBackdropMouseDown}
          aria-hidden="true"
        />
      ) : null}
      {isGlobalSearchPanelOpen && isMobile ? (
        <MobileSearchOverlay
          onClose={closeGlobalSearchPanel}
          onOpenOverview={onOpenOverview}
        />
      ) : null}
      <AddToCatalogDialog
        open={addDialogTarget !== null}
        onOpenChange={(open) => { if (!open) setAddDialogTarget(null); }}
        result={addDialogTarget?.result ?? EMPTY_SEARCH_RESULT}
        facet={addDialogTarget?.facet ?? "series"}
        catalogQualityProfileOptions={catalogQualityProfileOptions}
        catalogConfigLoading={Boolean(addDialogTarget) && catalogConfigLoading && !isAddDialogConfigReady}
        defaultQualityProfileId={resolveDefaultQualityProfileIdForFacet(addDialogTarget?.facet ?? "series")}
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
    </>
  );
});
