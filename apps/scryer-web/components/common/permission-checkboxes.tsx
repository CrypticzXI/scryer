import type * as React from "react";
import { ChevronDown } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import type { LibraryRecord } from "@/lib/types";
import {
  APP_PERMISSIONS,
  LIBRARY_PERMISSIONS,
  libraryPermissionShadowSource,
  libraryPermissionsWithRequestShadowing,
  type AppPermission,
  type LibraryPermission,
} from "@/lib/utils/permissions";
import { cn } from "@/lib/utils";

export type LibraryPermissionDrafts = Record<string, string[]>;

type PermissionOption = {
  value: string;
  label: string;
};

type FacetPermissionDropdownProps = {
  facet: LibraryRecord["facet"];
  label: string;
  libraries: LibraryRecord[];
  permissions?: string[];
  selectedByLibrary: LibraryPermissionDrafts;
  disabled?: boolean;
  idPrefix?: string;
  onLibraryChange: (libraryId: string, next: string[], permission: string) => void;
};

export const APP_PERMISSION_OPTIONS: Array<{ value: AppPermission; label: string }> = [
  { value: APP_PERMISSIONS.manageUsers, label: "Manage Users" },
  { value: APP_PERMISSIONS.managePermissions, label: "Manage Permissions" },
  { value: APP_PERMISSIONS.manageSystemSettings, label: "Manage System Settings" },
  { value: APP_PERMISSIONS.manageCatalogSettings, label: "Manage Catalog Settings" },
];

export const LIBRARY_PERMISSION_OPTIONS: Array<{ value: LibraryPermission; label: string }> = [
  { value: LIBRARY_PERMISSIONS.view, label: "View" },
  { value: LIBRARY_PERMISSIONS.manageTitles, label: "Manage Titles" },
  { value: LIBRARY_PERMISSIONS.resolveImports, label: "Resolve Imports" },
  { value: LIBRARY_PERMISSIONS.manageLibrary, label: "Manage Library" },
  { value: LIBRARY_PERMISSIONS.request, label: "Request" },
  { value: LIBRARY_PERMISSIONS.autoApproveRequests, label: "Auto-Approve Requests" },
];

const FACET_OPTIONS: Array<{ value: LibraryRecord["facet"]; label: string }> = [
  { value: "movie", label: "Movie" },
  { value: "series", label: "Series" },
  { value: "anime", label: "Anime" },
];

function permissionLabel(permission: string, options: PermissionOption[]) {
  return options.find((option) => option.value === permission)?.label ?? permission;
}

function permissionOptions(permissions: string[] | undefined, options: PermissionOption[]) {
  if (!permissions) {
    return options;
  }

  return permissions.map((permission) => ({
    value: permission,
    label: permissionLabel(permission, options),
  }));
}

export function togglePermissionValue(current: string[], value: string): string[] {
  const next = new Set(current);
  if (next.has(value)) {
    next.delete(value);
  } else {
    next.add(value);
  }
  return Array.from(next);
}

function shadowTitle(source: string | null): string | undefined {
  return source ? `Included by ${source}` : undefined;
}

function selectedCountLabel(count: number) {
  return count === 1 ? "1 selected" : `${count} selected`;
}

function PermissionDropdownTrigger({
  label,
  count,
  disabled,
  readOnly,
  className,
  ...props
}: React.ComponentProps<typeof Button> & {
  label: string;
  count: number;
  disabled?: boolean;
  readOnly?: boolean;
}) {
  return (
    <Button
      type="button"
      variant="outline"
      className={cn(
        "w-full justify-between bg-field px-3 text-left font-normal hover:bg-field/90",
        readOnly && "opacity-60",
        className,
      )}
      disabled={disabled}
      aria-disabled={disabled || readOnly}
      {...props}
    >
      <span
        className={cn(
          "min-w-0 truncate text-sm",
          count === 0 && "text-muted-foreground",
        )}
      >
        {count === 0 ? label : `${label}: ${selectedCountLabel(count)}`}
      </span>
      <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground" />
    </Button>
  );
}

function AppPermissionDropdown({
  permissions,
  selected,
  disabled,
  idPrefix,
  onChange,
}: {
  permissions?: string[];
  selected: string[];
  disabled?: boolean;
  idPrefix?: string;
  onChange: (next: string[], permission: string) => void;
}) {
  const options = permissionOptions(permissions, APP_PERMISSION_OPTIONS);

  return (
    <Popover>
      <PopoverTrigger asChild>
        <PermissionDropdownTrigger
          id={idPrefix ? `${idPrefix}-app-trigger` : undefined}
          label="App"
          count={selected.length}
          disabled={options.length === 0}
          readOnly={disabled}
        />
      </PopoverTrigger>
      <PopoverContent
        align="start"
        className="w-max min-w-[var(--radix-popover-trigger-width)] max-w-[calc(100vw-2rem)] p-2"
      >
        <div className="flex max-h-72 flex-col gap-1 overflow-y-auto">
          {options.map((permission) => {
            const checked = selected.includes(permission.value);
            return (
              <button
                id={idPrefix ? `${idPrefix}-app-${permission.value}` : undefined}
                key={permission.value}
                type="button"
                onClick={() =>
                  onChange(togglePermissionValue(selected, permission.value), permission.value)
                }
                disabled={disabled}
                className="flex min-w-max items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-60 disabled:hover:bg-transparent"
              >
                <Checkbox checked={checked} disabled={disabled} className="pointer-events-none" />
                <span className="whitespace-nowrap">{permission.label}</span>
              </button>
            );
          })}
        </div>
      </PopoverContent>
    </Popover>
  );
}

function FacetPermissionDropdown({
  facet,
  label,
  libraries,
  permissions,
  selectedByLibrary,
  disabled,
  idPrefix,
  onLibraryChange,
}: FacetPermissionDropdownProps) {
  const options = permissionOptions(permissions, LIBRARY_PERMISSION_OPTIONS);
  const facetLibraries = libraries.filter((library) => library.facet === facet);
  const selectedCount = facetLibraries.reduce(
    (count, library) =>
      count +
      libraryPermissionsWithRequestShadowing(selectedByLibrary[library.id] ?? []).length,
    0,
  );

  return (
    <Popover>
      <PopoverTrigger asChild>
        <PermissionDropdownTrigger
          id={idPrefix ? `${idPrefix}-${facet}-trigger` : undefined}
          label={label}
          count={selectedCount}
          disabled={facetLibraries.length === 0}
          readOnly={disabled}
        />
      </PopoverTrigger>
      <PopoverContent
        align="start"
        className="w-max min-w-[var(--radix-popover-trigger-width)] max-w-[calc(100vw-2rem)] p-2"
      >
        <div className="flex max-h-80 flex-col gap-3 overflow-y-auto">
          {facetLibraries.map((library) => {
            const selected = selectedByLibrary[library.id] ?? [];
            const effectiveSelected = libraryPermissionsWithRequestShadowing(selected);
            return (
              <div key={library.id} className="space-y-1">
                <div className="truncate px-2 text-xs font-semibold uppercase text-muted-foreground">
                  {library.name}
                </div>
                <div className="space-y-0.5">
                  {options.map((permission) => {
                    const checked = effectiveSelected.includes(permission.value);
                    const shadowSource = libraryPermissionShadowSource(
                      selected,
                      permission.value,
                    );
                    const permissionDisabled = disabled || shadowSource !== null;
                    return (
                      <button
                        id={
                          idPrefix
                            ? `${idPrefix}-${facet}-${library.id}-${permission.value}`
                            : undefined
                        }
                        key={`${library.id}-${permission.value}`}
                        type="button"
                        onClick={() =>
                          onLibraryChange(
                            library.id,
                            togglePermissionValue(selected, permission.value),
                            permission.value,
                          )
                        }
                        disabled={permissionDisabled}
                        title={shadowTitle(shadowSource)}
                        className="flex min-w-max items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-60 disabled:hover:bg-transparent"
                      >
                        <Checkbox
                          checked={checked}
                          disabled={permissionDisabled}
                          className="pointer-events-none"
                        />
                        <span className="whitespace-nowrap">{permission.label}</span>
                      </button>
                    );
                  })}
                </div>
              </div>
            );
          })}
        </div>
      </PopoverContent>
    </Popover>
  );
}

export function PermissionDropdowns({
  libraries,
  appPermissions,
  libraryPermissions,
  selectedAppPermissions,
  selectedLibraryPermissions,
  disabled,
  idPrefix,
  onAppChange,
  onLibraryChange,
}: {
  libraries: LibraryRecord[];
  appPermissions?: string[];
  libraryPermissions?: string[];
  selectedAppPermissions: string[];
  selectedLibraryPermissions: LibraryPermissionDrafts;
  disabled?: boolean;
  idPrefix?: string;
  onAppChange: (next: string[], permission: string) => void;
  onLibraryChange: (libraryId: string, next: string[], permission: string) => void;
}) {
  return (
    <div className="grid gap-2 sm:grid-cols-2 xl:grid-cols-4">
      <AppPermissionDropdown
        permissions={appPermissions}
        selected={selectedAppPermissions}
        disabled={disabled}
        idPrefix={idPrefix}
        onChange={onAppChange}
      />
      {FACET_OPTIONS.map((facet) => (
        <FacetPermissionDropdown
          key={facet.value}
          facet={facet.value}
          label={facet.label}
          libraries={libraries}
          permissions={libraryPermissions}
          selectedByLibrary={selectedLibraryPermissions}
          disabled={disabled}
          idPrefix={idPrefix}
          onLibraryChange={onLibraryChange}
        />
      ))}
    </div>
  );
}
