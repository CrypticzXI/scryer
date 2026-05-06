
import * as React from "react";
import {
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import { Loader2 } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { useNavigate } from "react-router-dom";
import { TitlePosterSlot } from "@/components/title-poster-slot";
import type { OverviewTitleTarget, ViewId } from "@/components/root/types";
import { sectionLabelForFacet, viewFromFacet } from "@/lib/facets/helpers";
import { FACET_REGISTRY } from "@/lib/facets/registry";
import { titlesQuery } from "@/lib/graphql/queries";
import { useTranslate } from "@/lib/context/translate-context";
import type { Facet, TitleRecord } from "@/lib/types";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";
import { buildOverviewDetailPath } from "@/lib/utils/routing";
import { useClient } from "urql";

export type RouteCommandItem = {
  id: string;
  label: string;
  description: string;
  icon?: LucideIcon;
  keywords?: string[];
  onSelect: () => void;
};

export type RouteCommandPaletteConfig = {
  title: string;
  description: string;
  placeholder: string;
  noResultsText: string;
  groupLabel: string;
  items: RouteCommandItem[];
};

type RouteCommandPaletteProps = {
  config?: RouteCommandPaletteConfig;
  onOpenOverview?: (targetView: ViewId, overviewTarget: OverviewTitleTarget) => void;
};

const CATALOG_COMMAND_MIN_QUERY_LENGTH = 2;
const CATALOG_COMMAND_RESULT_LIMIT = 8;

function catalogFacetFromString(facet: string): Facet {
  return facet === "movie" ? "movie" : facet === "anime" ? "anime" : "series";
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
      // Cmd+K / Ctrl+K
      if (event.key === "k" && (event.metaKey || event.ctrlKey)) {
        event.preventDefault();
        setOpen((prev) => !prev);
        return;
      }

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
    if (!open || query.length < CATALOG_COMMAND_MIN_QUERY_LENGTH) {
      setCatalogResults([]);
      setCatalogLoading(false);
      return;
    }

    const requestSeq = ++catalogRequestSeqRef.current;
    setCatalogLoading(true);
    const timer = window.setTimeout(() => {
      void client
        .query(titlesQuery, { facet: null, query }, { requestPolicy: "network-only" })
        .toPromise()
        .then(({ data, error }) => {
          if (requestSeq !== catalogRequestSeqRef.current) {
            return;
          }
          if (error) {
            throw error;
          }
          setCatalogResults(
            ((data?.titles ?? []) as TitleRecord[]).slice(
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
  }, [client, open, searchValue]);

  if (!config || config.items.length === 0) {
    return null;
  }

  const showCatalogBeforeNavigation =
    searchValue.trim().length >= CATALOG_COMMAND_MIN_QUERY_LENGTH &&
    (catalogLoading || catalogResults.length > 0);

  const catalogCommandGroup = (
    <CommandGroup heading={t("search.catalog")}>
      {catalogLoading ? (
        <CommandItem disabled value={`catalog-loading ${searchValue}`}>
          <div className="flex flex-1 items-center gap-2 text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            <span>{t("label.loading")}</span>
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
              <div className="h-10 w-7 flex-none overflow-hidden rounded-sm border border-border bg-muted">
                {hasPoster ? (
                  <TitlePosterSlot
                    src={posterUrl}
                    sourceSrc={title.posterSourceUrl}
                    metadataFetchedAt={title.metadataFetchedAt}
                    createdAt={title.createdAt}
                    alt={t("media.posterAlt", { name: title.name })}
                    className="h-full w-full object-cover"
                    placeholderClassName="flex h-full w-full items-center justify-center text-[10px] text-muted-foreground"
                    emptyLabel=""
                    loading="lazy"
                  />
                ) : Icon ? (
                  <div className="flex h-full w-full items-center justify-center text-muted-foreground">
                    <Icon className="h-3.5 w-3.5" />
                  </div>
                ) : null}
              </div>
              <span className="truncate">{title.name}</span>
              <span className="ml-auto shrink-0 text-xs text-muted-foreground">
                {sectionLabelForFacet(t, facet)}
                {year}
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
    >
      <CommandInput
        value={searchValue}
        onValueChange={setSearchValue}
        placeholder={config.placeholder}
      />
      <CommandList>
        <CommandEmpty>{config.noResultsText}</CommandEmpty>
        {showCatalogBeforeNavigation ? catalogCommandGroup : null}
        <CommandGroup heading={config.groupLabel}>
          {config.items.map((item) => (
            <CommandItem
              key={item.id}
              value={item.id}
              keywords={item.keywords}
              onSelect={() => handleCommandNavigate(item.onSelect)}
            >
              <div className="flex flex-1 items-center gap-2">
                {item.icon ? <item.icon className="h-4 w-4" /> : null}
                <span className="truncate">{item.label}</span>
                <span className="ml-auto text-xs text-muted-foreground">{item.description}</span>
              </div>
            </CommandItem>
          ))}
        </CommandGroup>
        {!showCatalogBeforeNavigation ? catalogCommandGroup : null}
      </CommandList>
    </CommandDialog>
  );
}
