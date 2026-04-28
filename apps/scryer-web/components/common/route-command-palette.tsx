
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
import type { OverviewTitleTarget, ViewId } from "@/components/root/types";
import { sectionLabelForFacet, viewFromFacet } from "@/lib/facets/helpers";
import { FACET_REGISTRY } from "@/lib/facets/registry";
import { titlesQuery } from "@/lib/graphql/queries";
import { useTranslate } from "@/lib/context/translate-context";
import type { Facet, TitleRecord } from "@/lib/types";
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
  const t = useTranslate();

  const handleCommandNavigate = React.useCallback((callback: () => void) => {
    setOpen(false);
    callback();
  }, []);

  const handleCatalogTitleSelect = React.useCallback(
    (title: TitleRecord) => {
      setOpen(false);
      onOpenOverview?.(viewFromFacet(catalogFacetFromString(title.facet)), {
        id: title.id,
        slug: title.slug ?? null,
      });
    },
    [onOpenOverview],
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
    if (!open || !onOpenOverview || query.length < CATALOG_COMMAND_MIN_QUERY_LENGTH) {
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
  }, [client, onOpenOverview, open, searchValue]);

  if (!config || config.items.length === 0) {
    return null;
  }

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
        {onOpenOverview ? (
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
                  <div className="flex flex-1 items-center gap-2">
                    {Icon ? <Icon className="h-4 w-4" /> : null}
                    <span className="truncate">{title.name}</span>
                    <span className="ml-auto text-xs text-muted-foreground">
                      {sectionLabelForFacet(t, facet)}
                      {year}
                    </span>
                  </div>
                </CommandItem>
              );
            })}
          </CommandGroup>
        ) : null}
      </CommandList>
    </CommandDialog>
  );
}
