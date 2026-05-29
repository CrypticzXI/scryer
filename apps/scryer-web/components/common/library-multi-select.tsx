import * as React from "react";
import { ChevronDown } from "lucide-react";

import { localizedFacetLabel } from "@/components/views/overview-localization";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { useTranslate } from "@/lib/context/translate-context";
import type { LibraryRecord } from "@/lib/types";
import { cn } from "@/lib/utils";
import { normalizeLibraryFilterSelection } from "@/lib/utils/library-filter";

const FACET_ORDER: LibraryRecord["facet"][] = ["movie", "series", "anime"];
const MAX_INLINE_LIBRARY_LABELS = 2;

type LibraryMultiSelectProps = {
  libraries: LibraryRecord[];
  selectedLibraryIds: string[];
  onSelectedLibraryIdsChange: (libraryIds: string[]) => void;
  disabled?: boolean;
  triggerId?: string;
  allLibrariesButtonId?: string;
  triggerClassName?: string;
  contentClassName?: string;
};

function groupLibrariesByFacet(libraries: LibraryRecord[]) {
  return FACET_ORDER
    .map((facet) => ({
      facet,
      libraries: libraries.filter((library) => library.facet === facet),
    }))
    .filter((group) => group.libraries.length > 0);
}

export function LibraryMultiSelect({
  libraries,
  selectedLibraryIds,
  onSelectedLibraryIdsChange,
  disabled = false,
  triggerId,
  allLibrariesButtonId,
  triggerClassName,
  contentClassName,
}: LibraryMultiSelectProps) {
  const t = useTranslate();

  const normalizedSelectedLibraryIds = React.useMemo(
    () => normalizeLibraryFilterSelection(selectedLibraryIds, libraries),
    [libraries, selectedLibraryIds],
  );
  const groupedLibraries = React.useMemo(
    () => groupLibrariesByFacet(libraries),
    [libraries],
  );
  const showFacetGroups = groupedLibraries.length > 1;
  const libraryById = React.useMemo(
    () => new Map(libraries.map((library) => [library.id, library])),
    [libraries],
  );
  const selectedLibraries = React.useMemo(
    () =>
      normalizedSelectedLibraryIds
        .map((libraryId) => libraryById.get(libraryId) ?? null)
        .filter((library): library is LibraryRecord => library !== null),
    [libraryById, normalizedSelectedLibraryIds],
  );
  const triggerLabel = React.useMemo(() => {
    if (selectedLibraries.length === 0) {
      return t("libraryFilter.all");
    }

    if (selectedLibraries.length <= MAX_INLINE_LIBRARY_LABELS) {
      return selectedLibraries
        .map((library) =>
          showFacetGroups
            ? `${localizedFacetLabel(t, library.facet)}: ${library.name}`
            : library.name,
        )
        .join(", ");
    }

    return t("libraryFilter.selectedCount", {
      count: selectedLibraries.length,
    });
  }, [selectedLibraries, showFacetGroups, t]);
  const implicitAllSelected = libraries.length > 0 && normalizedSelectedLibraryIds.length === 0;

  const toggleAllLibraries = React.useCallback(() => {
    onSelectedLibraryIdsChange([]);
  }, [onSelectedLibraryIdsChange]);

  const toggleLibrary = React.useCallback(
    (libraryId: string) => {
      if (normalizedSelectedLibraryIds.length === 0) {
        onSelectedLibraryIdsChange([libraryId]);
        return;
      }

      const allLibraryIds = libraries.map((library) => library.id);
      const selectedSet = new Set(
        normalizedSelectedLibraryIds.length > 0
          ? normalizedSelectedLibraryIds
          : allLibraryIds,
      );

      if (selectedSet.has(libraryId)) {
        selectedSet.delete(libraryId);
      } else {
        selectedSet.add(libraryId);
      }

      const nextSelection = allLibraryIds.filter((id) => selectedSet.has(id));
      onSelectedLibraryIdsChange(
        nextSelection.length === 0 || nextSelection.length === allLibraryIds.length
          ? []
          : nextSelection,
      );
    },
    [libraries, normalizedSelectedLibraryIds, onSelectedLibraryIdsChange],
  );

  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button
          id={triggerId}
          type="button"
          variant="outline"
          className={cn(
            "justify-between bg-field px-3 text-left font-normal hover:bg-field/90",
            triggerClassName,
          )}
          disabled={disabled || libraries.length === 0}
        >
          <span
            className={cn(
              "truncate",
              normalizedSelectedLibraryIds.length === 0 && "text-muted-foreground",
            )}
          >
            {triggerLabel}
          </span>
          <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground" />
        </Button>
      </PopoverTrigger>
      <PopoverContent
        align="start"
        className={cn("w-[var(--radix-popover-trigger-width)] p-2", contentClassName)}
      >
        <div className="flex max-h-80 flex-col gap-1 overflow-y-auto">
          <button
            id={allLibrariesButtonId}
            type="button"
            onClick={toggleAllLibraries}
            className="flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm transition-colors hover:bg-accent"
          >
            <Checkbox
              checked={implicitAllSelected}
              className="pointer-events-none"
            />
            <span className="truncate">{t("libraryFilter.all")}</span>
          </button>

          {groupedLibraries.map((group) => (
            <div key={group.facet} className="space-y-1">
              {showFacetGroups ? (
                <div className="px-2 pt-1 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                  {localizedFacetLabel(t, group.facet)}
                </div>
              ) : null}
              {group.libraries.map((library) => {
                const checked =
                  implicitAllSelected || normalizedSelectedLibraryIds.includes(library.id);
                const implicitChecked =
                  implicitAllSelected && !normalizedSelectedLibraryIds.includes(library.id);
                return (
                  <button
                    key={library.id}
                    type="button"
                    onClick={() => toggleLibrary(library.id)}
                    className="flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm transition-colors hover:bg-accent"
                  >
                    <Checkbox
                      checked={checked}
                      className={cn(
                        "pointer-events-none",
                        implicitChecked &&
                          "data-[state=checked]:border-muted-foreground/30 data-[state=checked]:bg-muted data-[state=checked]:text-muted-foreground",
                      )}
                    />
                    <span
                      className={cn(
                        "truncate",
                        implicitChecked && "text-muted-foreground",
                      )}
                    >
                      {library.name}
                    </span>
                  </button>
                );
              })}
            </div>
          ))}
        </div>
      </PopoverContent>
    </Popover>
  );
}
