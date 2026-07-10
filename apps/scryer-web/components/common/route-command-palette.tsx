
import * as React from "react";
import {
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import { ArrowRight, Loader2 } from "lucide-react";
import { useNavigate } from "react-router-dom";
import { TitlePosterSlot } from "@/components/title-poster-slot";
import type { OverviewTitleTarget, ViewId } from "@/components/root/types";
import { sectionLabelForFacet, viewFromFacet } from "@/lib/facets/helpers";
import { FACET_REGISTRY } from "@/lib/facets/registry";
import { commandPaletteTitlesQuery } from "@/lib/graphql/queries";
import { useTranslate } from "@/lib/context/translate-context";
import type { Facet, TitleRecord } from "@/lib/types";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";
import { buildOverviewDetailPath } from "@/lib/utils/routing";
import { useAuth } from "@/lib/hooks/use-auth";
import {
  LIBRARY_PERMISSIONS,
  hasAnyLibraryPermission,
} from "@/lib/utils/permissions";
import { useClient } from "urql";
import {
  filterRouteCommandItems,
  groupRouteCommandItems,
  routeCommandDisplayLabel,
  type RouteCommandPaletteConfig,
} from "@/components/common/route-command-types";

type RouteCommandPaletteProps = {
  config?: RouteCommandPaletteConfig;
  onOpenOverview?: (targetView: ViewId, overviewTarget: OverviewTitleTarget) => void;
};

const CATALOG_COMMAND_MIN_QUERY_LENGTH = 2;
const CATALOG_COMMAND_RESULT_LIMIT = 8;

function catalogFacetFromString(facet: string): Facet {
  return facet === "movie" ? "MOVIE" : facet === "anime" ? "ANIME" : "SERIES";
}

function isTextInput(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) {
    return false;
  }
  const tag = target.tagName.toLowerCase();
  return (
    target.isContentEditable ||
    tag === "input" ||
    tag === "textarea" ||
    tag === "select"
  );
}

export function RouteCommandPalette({
  config,
  onOpenOverview,
}: RouteCommandPaletteProps) {
  const [open, setOpen] = React.useState(false);
  const [searchValue, setSearchValue] = React.useState("");
  const [catalogResults, setCatalogResults] = React.useState<TitleRecord[]>([]);
  const [catalogLoading, setCatalogLoading] = React.useState(false);
  const lastShiftPressAt = React.useRef(0);
  const catalogRequestSeqRef = React.useRef(0);
  const client = useClient();
  const navigate = useNavigate();
  const t = useTranslate();
  const { user } = useAuth();
  const canViewCatalog = user
    ? hasAnyLibraryPermission(user, LIBRARY_PERMISSIONS.view)
    : false;

  const handleCommandNavigate = React.useCallback((callback: () => void) => {
    setOpen(false);
    callback();
  }, []);

  const handleOpenOverviewTarget = React.useCallback(
    (targetView: ViewId, overviewTarget: OverviewTitleTarget) => {
      if (onOpenOverview) {
        onOpenOverview(targetView, overviewTarget);
        return;
      }

      const normalizedTitleId = overviewTarget.id.trim();
      if (!normalizedTitleId) {
        return;
      }

      const normalizedSlug = overviewTarget.slug?.trim() || null;
      const normalizedLibrarySlug = overviewTarget.librarySlug?.trim() || null;
      const targetPath = buildOverviewDetailPath(targetView, normalizedLibrarySlug, normalizedSlug);
      const nextParams = new URLSearchParams();
      if (!normalizedSlug || !normalizedLibrarySlug) {
        nextParams.set("id", normalizedTitleId);
      }

      const nextQuery = nextParams.toString();
      navigate(`${targetPath}${nextQuery ? `?${nextQuery}` : ""}`);
    },
    [navigate, onOpenOverview],
  );

  const handleCatalogTitleSelect = React.useCallback(
    (title: TitleRecord) => {
      setOpen(false);
      handleOpenOverviewTarget(viewFromFacet(catalogFacetFromString(title.facet)), {
        id: title.id,
        slug: title.slug ?? null,
        libraryId: title.libraryId,
        librarySlug: title.librarySlug ?? null,
      });
    },
    [handleOpenOverviewTarget],
  );

  React.useEffect(() => {
    if (!config || config.items.length === 0) {
      return undefined;
    }

    const onKeyDown = (event: KeyboardEvent) => {
      // Double-Shift
      if (event.key !== "Shift" || event.repeat || isTextInput(event.target)) {
        return;
      }

      const now = performance.now();
      const previousShiftPressAt = lastShiftPressAt.current;

      if (previousShiftPressAt && now - previousShiftPressAt < 300) {
        setOpen(true);
        lastShiftPressAt.current = 0;
        return;
      }

      lastShiftPressAt.current = now;
    };

    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      lastShiftPressAt.current = 0;
    };
  }, [config]);

  React.useEffect(() => {
    if (open) {
      return;
    }

    setSearchValue("");
    setCatalogResults([]);
    setCatalogLoading(false);
    catalogRequestSeqRef.current += 1;
  }, [open]);

  React.useEffect(() => {
    const query = searchValue.trim();
    if (!open || !canViewCatalog || query.length < CATALOG_COMMAND_MIN_QUERY_LENGTH) {
      catalogRequestSeqRef.current += 1;
      setCatalogResults([]);
      setCatalogLoading(false);
      return;
    }

    const requestSeq = ++catalogRequestSeqRef.current;
    setCatalogLoading(true);
    const timer = window.setTimeout(() => {
      void client
        .query(commandPaletteTitlesQuery, { facet: null, query, limit: CATALOG_COMMAND_RESULT_LIMIT }, { requestPolicy: "network-only" })
        .toPromise()
        .then(({ data, error }) => {
          if (requestSeq !== catalogRequestSeqRef.current) {
            return;
          }
          if (error) {
            throw error;
          }
          setCatalogResults(
            ((data?.titles?.items ?? []) as TitleRecord[]).slice(
              0,
              CATALOG_COMMAND_RESULT_LIMIT,
            ),
          );
        })
        .catch(() => {
          if (requestSeq === catalogRequestSeqRef.current) {
            setCatalogResults([]);
          }
        })
        .finally(() => {
          if (requestSeq === catalogRequestSeqRef.current) {
            setCatalogLoading(false);
          }
        });
    }, 150);

    return () => {
      window.clearTimeout(timer);
    };
  }, [canViewCatalog, client, open, searchValue]);

  const routeCommandItems = config?.items;
  const routeCommandResults = React.useMemo(
    () => filterRouteCommandItems(routeCommandItems ?? [], searchValue),
    [routeCommandItems, searchValue],
  );

  if (!config || config.items.length === 0) {
    return null;
  }

  const showCatalogBeforeNavigation =
    canViewCatalog &&
    searchValue.trim().length >= CATALOG_COMMAND_MIN_QUERY_LENGTH &&
    (catalogLoading || catalogResults.length > 0);

  const catalogCommandGroup = (
    <CommandGroup heading={t("search.catalog")}>
      {catalogLoading ? (
        <CommandItem disabled value={`catalog-loading ${searchValue}`}>
          <div className="flex min-h-10 flex-1 items-center gap-3 text-[var(--scry-muted3)]">
            <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-[8px] bg-[var(--scry-chip)] text-[var(--scry-accent-ring)]">
              <Loader2 className="h-4 w-4 animate-spin" />
            </span>
            <span className="text-sm font-medium">{t("label.loading")}</span>
          </div>
        </CommandItem>
      ) : null}
      {catalogResults.map((title) => {
        const facet = catalogFacetFromString(title.facet);
        const registryEntry = FACET_REGISTRY.find((item) => item.id === facet);
        const Icon = registryEntry?.icon;
        const posterUrl = selectPosterVariantUrl(title.posterUrl, "w70");
        const hasPoster = Boolean(posterUrl || title.posterSourceUrl);
        const year = title.year ? ` • ${title.year}` : "";
        return (
          <CommandItem
            key={`catalog-title-${title.id}`}
            value={`catalog-title-${title.id} ${title.name} ${searchValue}`}
            keywords={[
              searchValue,
              title.name,
              title.slug ?? "",
              title.sortTitle ?? "",
              facet,
            ]}
            onSelect={() => handleCatalogTitleSelect(title)}
          >
            <div className="flex min-w-0 flex-1 items-center gap-3">
              <div className="h-12 w-8 flex-none overflow-hidden rounded-[7px] border border-[var(--scry-border2)] bg-[var(--scry-chip)]">
                {hasPoster ? (
                  <TitlePosterSlot
                    src={posterUrl}
                    sourceSrc={title.posterSourceUrl}
                    metadataFetchedAt={title.metadataFetchedAt}
                    createdAt={title.createdAt}
                    alt={t("media.posterAlt", { name: title.name })}
                    className="h-full w-full object-cover"
                    placeholderClassName="flex h-full w-full items-center justify-center text-[10px] text-[var(--scry-faint2)]"
                    emptyLabel=""
                    loading="lazy"
                  />
                ) : Icon ? (
                  <div className="flex h-full w-full items-center justify-center text-[var(--scry-faint2)]">
                    <Icon className="h-3.5 w-3.5" />
                  </div>
                ) : null}
              </div>
              <span className="min-w-0 flex-1">
                <span className="block truncate text-sm font-semibold text-[var(--scry-ink2)]">
                  {title.name}
                </span>
                <span className="mt-0.5 block truncate text-xs text-[var(--scry-muted3)]">
                  {sectionLabelForFacet(t, facet)}
                  {year}
                </span>
              </span>
              <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-[8px] text-[var(--scry-faint2)]">
                <ArrowRight className="h-3.5 w-3.5" />
              </span>
            </div>
          </CommandItem>
        );
      })}
    </CommandGroup>
  );

  return (
    <CommandDialog
      open={open}
      onOpenChange={setOpen}
      title={config.title}
      description={config.description}
      showCloseButton={false}
      contentId="route-command-palette"
    >
      <CommandInput
        id="route-command-palette-input"
        wrapperId="route-command-palette-input-wrapper"
        value={searchValue}
        onValueChange={setSearchValue}
        placeholder={config.placeholder}
      />
      <CommandList>
        <CommandEmpty>{config.noResultsText}</CommandEmpty>
        {showCatalogBeforeNavigation ? catalogCommandGroup : null}
        {groupRouteCommandItems(routeCommandResults).map((group) => (
          <CommandGroup
            key={group.groupLabel ?? "route-command-ungrouped"}
            heading={group.groupLabel ?? config.groupLabel}
          >
            {group.items.map((item) => {
              const displayLabel = routeCommandDisplayLabel(item);
              return (
                <CommandItem
                  id={`route-command-item-${item.id}`}
                  key={item.id}
                  value={item.id}
                  keywords={[
                    item.label,
                    displayLabel,
                    item.description,
                    item.groupLabel ?? "",
                    ...(item.keywords ?? []),
                  ]}
                  onSelect={() => handleCommandNavigate(item.onSelect)}
                >
                  <div className="flex min-w-0 flex-1 items-center gap-3">
                    {item.icon ? (
                      <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-[8px] border border-[rgba(var(--scry-accent-rgb),0.18)] bg-[rgba(var(--scry-accent-rgb),0.12)] text-[var(--scry-accent-text)]">
                        <item.icon className="h-4 w-4" />
                      </span>
                    ) : null}
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-sm font-semibold text-[var(--scry-ink2)]">
                        {displayLabel}
                      </span>
                      {item.description ? (
                        <span className="mt-0.5 block truncate text-xs text-[var(--scry-muted3)]">
                          {item.description}
                        </span>
                      ) : null}
                    </span>
                    <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-[8px] text-[var(--scry-faint2)]">
                      <ArrowRight className="h-3.5 w-3.5" />
                    </span>
                  </div>
                </CommandItem>
              );
            })}
          </CommandGroup>
        ))}
      </CommandList>
    </CommandDialog>
  );
}
