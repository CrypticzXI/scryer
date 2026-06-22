import * as React from "react";
import { FolderOpen, Pencil, Plus, RefreshCw, Save, Trash2 } from "lucide-react";
import { SubtitleLanguagePicker } from "@/components/common/subtitle-language-picker";
import { FolderBrowserDialog } from "@/components/setup/folder-browser-dialog";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useTranslate } from "@/lib/context/translate-context";
import { SCORING_PERSONA_CHOICES } from "@/lib/constants/quality-profiles";
import { cn } from "@/lib/utils";
import { selectorId } from "@/lib/utils/dom-ids";
import { DownloadClientRoutingPanel } from "@/components/views/media-content/download-client-routing-panel";
import {
  boxedActionButtonBaseClass,
  boxedActionButtonToneClass,
} from "@/lib/utils/action-button-styles";
import type {
  DownloadClientRecord,
  DownloadClientRoutingEntry,
  LibraryRecord,
  LibraryScanSummary,
  LibrarySettingsDraft,
  LibrarySettingsRecord,
  ParsedQualityProfile,
  RootFolderOption,
  ScoringPersonaId,
} from "@/lib/types";
import type { ImportMode } from "@/lib/types/settings";
import type { DownloadClientRoutingSettingsByClient } from "@/lib/types/download-clients";
import {
  buildDownloadClientRoutingState,
  disabledDownloadClientRoutingSettings,
  serializeDownloadClientRoutingEntries,
} from "@/lib/utils/download-client-routing";
import {
  areNzbgetRoutingMapsEqual,
  areRoutingOrdersEqual,
} from "@/lib/utils/media-content";
import {
  isLocalPathFormatValidForStyle,
  type LocalPathStyle,
} from "@/lib/utils/local-path-style";

const INHERIT_VALUE = "__inherit__";
const BOOLEAN_TRUE_VALUE = "true";
const BOOLEAN_FALSE_VALUE = "false";
const FILLER_POLICY_OPTIONS = [
  { value: "download_all", labelKey: "settings.fillerPolicyDownloadAll" },
  { value: "skip_filler", labelKey: "settings.fillerPolicySkipFiller" },
] as const;
const RECAP_POLICY_OPTIONS = [
  { value: "download_all", labelKey: "settings.recapPolicyDownloadAll" },
  { value: "skip_recap", labelKey: "settings.recapPolicySkipRecap" },
] as const;
const BOOLEAN_OVERRIDE_OPTIONS = [
  { value: INHERIT_VALUE, labelKey: "settings.libraryInheritFacet" },
  { value: BOOLEAN_TRUE_VALUE, labelKey: "label.enabled" },
  { value: BOOLEAN_FALSE_VALUE, labelKey: "label.disabled" },
] as const;
const IMPORT_MODE_OPTIONS = [
  { value: INHERIT_VALUE, labelKey: "settings.libraryInheritFacet" },
  { value: "hardlink_or_copy", labelKey: "settings.importModeHardlinkCopy" },
  { value: "move", labelKey: "settings.importModeMove" },
] as const;

type LibraryMutationInput = {
  name: string;
  roots: RootFolderOption[];
  settings?: LibrarySettingsDraft;
};

type MediaLibrarySettingsPanelProps = {
  facet: LibraryRecord["facet"];
  settingsTitle: string;
  libraries: LibraryRecord[];
  librariesLoading: boolean;
  rootValidationLibraries: LibraryRecord[];
  rootValidationLibrariesLoading: boolean;
  preferredLibraryId: string;
  allLibrariesValue: string;
  loading: boolean;
  saving: boolean;
  scanLoading: boolean;
  scanNotice?: string | null;
  scanSummary: LibraryScanSummary | null;
  localPathStyle: LocalPathStyle | undefined;
  qualityProfiles: ParsedQualityProfile[];
  downloadClients: DownloadClientRecord[];
  downloadClientsLoading: boolean;
  loadLibrarySettings: (libraryId: string) => Promise<LibrarySettingsRecord | null>;
  loadFacetDownloadClientRouting: (
    scopeId: LibraryRecord["facet"],
  ) => Promise<DownloadClientRoutingEntry[]>;
  onCreateLibrary: (input: LibraryMutationInput) => Promise<LibraryRecord | null | void> | LibraryRecord | null | void;
  onUpdateLibrary: (libraryId: string, input: LibraryMutationInput) => Promise<LibraryRecord | null | void> | LibraryRecord | null | void;
  onDeleteLibrary: (libraryId: string) => Promise<boolean | void> | boolean | void;
  onScan: (libraryId: string) => Promise<void> | void;
};

const NEW_LIBRARY_VALUE = "__new_library__";

function rootsFromLibrary(library: LibraryRecord | null): RootFolderOption[] {
  return (library?.roots ?? []).map((root) => ({
    id: root.id,
    path: root.path,
    isDefault: root.isDefault,
  }));
}

function normalizeComparableRootPath(path: string): string {
  return path.trim().replace(/\/+$/u, "").toLowerCase();
}

function normalizeRoots(roots: RootFolderOption[]): RootFolderOption[] {
  const seen = new Set<string>();
  let hasDefault = false;
  const next: RootFolderOption[] = [];

  roots.forEach((root) => {
    const path = root.path.trim();
    if (!path || seen.has(path)) {
      return;
    }
    seen.add(path);
    const isDefault = root.isDefault && !hasDefault;
    if (isDefault) {
      hasDefault = true;
    }
    next.push({ id: root.id, path, isDefault });
  });

  if (next.length > 0 && !hasDefault) {
    next[0] = { ...next[0], isDefault: true };
  }

  return next;
}

function rootsEqual(left: RootFolderOption[], right: RootFolderOption[]): boolean {
  const normalizedLeft = normalizeRoots(left);
  const normalizedRight = normalizeRoots(right);
  if (normalizedLeft.length !== normalizedRight.length) {
    return false;
  }
  return normalizedLeft.every((root, index) => {
    const other = normalizedRight[index];
    return other && root.path === other.path && root.isDefault === other.isDefault;
  });
}

function booleanOverrideSelectValue(value: boolean | null | undefined): string {
  if (value == null) {
    return INHERIT_VALUE;
  }

  return value ? BOOLEAN_TRUE_VALUE : BOOLEAN_FALSE_VALUE;
}

function booleanOverrideFromSelectValue(value: string): boolean | null {
  if (value === INHERIT_VALUE) {
    return null;
  }

  return value === BOOLEAN_TRUE_VALUE;
}

function fillerPolicyLabelKey(value: string | null | undefined): string {
  return value === "skip_filler"
    ? "settings.fillerPolicySkipFiller"
    : "settings.fillerPolicyDownloadAll";
}

function recapPolicyLabelKey(value: string | null | undefined): string {
  return value === "skip_recap"
    ? "settings.recapPolicySkipRecap"
    : "settings.recapPolicyDownloadAll";
}

function importModeLabelKey(value: ImportMode | null | undefined): string {
  return value === "move"
    ? "settings.importModeMove"
    : "settings.importModeHardlinkCopy";
}

export const MediaLibrarySettingsPanel = React.memo(function MediaLibrarySettingsPanel({
  facet,
  settingsTitle,
  libraries,
  librariesLoading,
  rootValidationLibraries,
  rootValidationLibrariesLoading,
  preferredLibraryId,
  allLibrariesValue,
  loading,
  saving,
  scanLoading,
  scanNotice,
  scanSummary,
  localPathStyle,
  qualityProfiles,
  downloadClients,
  downloadClientsLoading,
  loadLibrarySettings,
  loadFacetDownloadClientRouting,
  onCreateLibrary,
  onUpdateLibrary,
  onDeleteLibrary,
  onScan,
}: MediaLibrarySettingsPanelProps) {
  const t = useTranslate();
  const [mode, setMode] = React.useState<"existing" | "new">("existing");
  const [activeLibraryId, setActiveLibraryId] = React.useState<string | null>(null);
  const [draftName, setDraftName] = React.useState("");
  const [draftRoots, setDraftRoots] = React.useState<RootFolderOption[]>([]);
  const [settingsLoading, setSettingsLoading] = React.useState(false);
  const [settingsError, setSettingsError] = React.useState<string | null>(null);
  const [draftRequiredAudioLanguages, setDraftRequiredAudioLanguages] = React.useState<string[]>([]);
  const [draftQualityProfileId, setDraftQualityProfileId] = React.useState(INHERIT_VALUE);
  const [draftRequestQualityProfileIds, setDraftRequestQualityProfileIds] = React.useState<string[]>([]);
  const [draftScoringPersona, setDraftScoringPersona] = React.useState(INHERIT_VALUE);
  const [draftFillerPolicy, setDraftFillerPolicy] = React.useState(INHERIT_VALUE);
  const [draftRecapPolicy, setDraftRecapPolicy] = React.useState(INHERIT_VALUE);
  const [draftMonitorSpecials, setDraftMonitorSpecials] = React.useState(INHERIT_VALUE);
  const [draftInterSeasonMovies, setDraftInterSeasonMovies] = React.useState(INHERIT_VALUE);
  const [draftMonitorFillerMovies, setDraftMonitorFillerMovies] = React.useState(INHERIT_VALUE);
  const [draftNfoWriteOnImport, setDraftNfoWriteOnImport] = React.useState(INHERIT_VALUE);
  const [draftPlexmatchWriteOnImport, setDraftPlexmatchWriteOnImport] = React.useState(INHERIT_VALUE);
  const [draftImportMode, setDraftImportMode] = React.useState(INHERIT_VALUE);
  const [draftDownloadClientRoutingMode, setDraftDownloadClientRoutingMode] =
    React.useState<"inherit" | "custom">("inherit");
  const [draftDownloadClientRouting, setDraftDownloadClientRouting] =
    React.useState<DownloadClientRoutingSettingsByClient>({});
  const [draftDownloadClientRoutingOrder, setDraftDownloadClientRoutingOrder] =
    React.useState<string[]>([]);
  const [draftDownloadClientRoutingLoading, setDraftDownloadClientRoutingLoading] =
    React.useState(false);
  const [savedSettings, setSavedSettings] = React.useState<LibrarySettingsRecord | null>(null);
  const [browserOpen, setBrowserOpen] = React.useState(false);
  const [editingIndex, setEditingIndex] = React.useState<number | null>(null);
  const lastHydratedRoutingKeyRef = React.useRef<string | null>(null);

  const activeLibrary = React.useMemo(
    () => libraries.find((library) => library.id === activeLibraryId) ?? null,
    [activeLibraryId, libraries],
  );
  const currentFacet = activeLibrary?.facet ?? facet;
  const isAnimeFacet = currentFacet === "anime";
  const showPlexmatch = currentFacet === "series" || currentFacet === "anime";
  const savedDownloadClientRoutingEntries =
    savedSettings?.downloadClientRoutingOverride ?? null;
  const savedDownloadClientRoutingState = React.useMemo(
    () =>
      buildDownloadClientRoutingState(
        downloadClients,
        savedDownloadClientRoutingEntries ?? [],
        disabledDownloadClientRoutingSettings(),
      ),
    [downloadClients, savedDownloadClientRoutingEntries],
  );

  const hydrateSavedSettings = React.useCallback(
    (settings: LibrarySettingsRecord | null) => {
      setSavedSettings(settings);
      setDraftRequiredAudioLanguages(settings?.requiredAudioLanguagesOverride ?? []);
      setDraftQualityProfileId(settings?.qualityProfileIdOverride ?? INHERIT_VALUE);
      setDraftRequestQualityProfileIds(settings?.requestQualityProfileIdsOverride ?? []);
      setDraftScoringPersona(settings?.scoringPersonaOverride ?? INHERIT_VALUE);
      setDraftFillerPolicy(settings?.fillerPolicyOverride ?? INHERIT_VALUE);
      setDraftRecapPolicy(settings?.recapPolicyOverride ?? INHERIT_VALUE);
      setDraftMonitorSpecials(booleanOverrideSelectValue(settings?.monitorSpecialsOverride));
      setDraftInterSeasonMovies(
        booleanOverrideSelectValue(settings?.interSeasonMoviesOverride),
      );
      setDraftMonitorFillerMovies(
        booleanOverrideSelectValue(settings?.monitorFillerMoviesOverride),
      );
      setDraftNfoWriteOnImport(
        booleanOverrideSelectValue(settings?.nfoWriteOnImportOverride),
      );
      setDraftPlexmatchWriteOnImport(
        booleanOverrideSelectValue(settings?.plexmatchWriteOnImportOverride),
      );
      setDraftImportMode(settings?.importModeOverride ?? INHERIT_VALUE);
    },
    [],
  );

  React.useEffect(() => {
    if (mode === "new") {
      return;
    }
    if (libraries.length === 0) {
      setActiveLibraryId(null);
      return;
    }
    const preferred =
      preferredLibraryId !== allLibrariesValue
        ? libraries.find((library) => library.id === preferredLibraryId) ?? null
        : null;
    setActiveLibraryId((current) => {
      if (preferred) {
        return preferred.id;
      }
      if (current && libraries.some((library) => library.id === current)) {
        return current;
      }
      return libraries[0]?.id ?? null;
    });
  }, [allLibrariesValue, libraries, mode, preferredLibraryId]);

  React.useEffect(() => {
    if (mode === "new") {
      setSavedSettings(null);
      setDraftRequiredAudioLanguages([]);
      setDraftQualityProfileId(INHERIT_VALUE);
      setDraftRequestQualityProfileIds([]);
      setDraftScoringPersona(INHERIT_VALUE);
      setDraftFillerPolicy(INHERIT_VALUE);
      setDraftRecapPolicy(INHERIT_VALUE);
      setDraftMonitorSpecials(INHERIT_VALUE);
      setDraftInterSeasonMovies(INHERIT_VALUE);
      setDraftMonitorFillerMovies(INHERIT_VALUE);
      setDraftNfoWriteOnImport(INHERIT_VALUE);
      setDraftPlexmatchWriteOnImport(INHERIT_VALUE);
      setDraftImportMode(INHERIT_VALUE);
      setDraftDownloadClientRoutingMode("inherit");
      setDraftDownloadClientRouting({});
      setDraftDownloadClientRoutingOrder([]);
      setDraftDownloadClientRoutingLoading(false);
      return;
    }
    setDraftName(activeLibrary?.name ?? "");
    setDraftRoots(rootsFromLibrary(activeLibrary));
  }, [activeLibrary, mode]);

  React.useEffect(() => {
    let cancelled = false;
    if (!activeLibrary || mode === "new") {
      return () => {
        cancelled = true;
      };
    }

    setSettingsLoading(true);
    setSettingsError(null);
    void loadLibrarySettings(activeLibrary.id)
      .then((settings) => {
        if (cancelled) {
          return;
        }
        hydrateSavedSettings(settings);
      })
      .catch((error) => {
        if (!cancelled) {
          setSettingsError(error instanceof Error ? error.message : t("settings.librarySettingsLoadFailed"));
        }
      })
      .finally(() => {
        if (!cancelled) {
          setSettingsLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [activeLibrary, hydrateSavedSettings, loadLibrarySettings, mode, t]);

  React.useEffect(() => {
    const routingHydrationKey =
      mode === "new"
        ? `new:${currentFacet}:${downloadClients.map((client) => client.id).join(",")}`
        : `library:${activeLibrary?.id ?? "none"}:${savedDownloadClientRoutingEntries ? "custom" : "inherit"}:${(savedDownloadClientRoutingEntries ?? []).map((entry) => entry.clientId).join(",")}:${downloadClients.map((client) => client.id).join(",")}`;

    if (lastHydratedRoutingKeyRef.current === routingHydrationKey) {
      return;
    }
    lastHydratedRoutingKeyRef.current = routingHydrationKey;

    if (mode === "new") {
      setDraftDownloadClientRoutingMode("inherit");
      setDraftDownloadClientRouting({});
      setDraftDownloadClientRoutingOrder([]);
      return;
    }

    setDraftDownloadClientRoutingMode(
      savedDownloadClientRoutingEntries ? "custom" : "inherit",
    );
    setDraftDownloadClientRouting(savedDownloadClientRoutingState.routing);
    setDraftDownloadClientRoutingOrder(savedDownloadClientRoutingState.order);
  }, [
    activeLibrary?.id,
    currentFacet,
    downloadClients,
    mode,
    savedDownloadClientRoutingEntries,
    savedDownloadClientRoutingState,
  ]);

  const normalizedDraftRoots = React.useMemo(
    () => normalizeRoots(draftRoots),
    [draftRoots],
  );
  const conflictingLibraryNamesByRootPath = React.useMemo(() => {
    const otherLibrariesByRootPath = new Map<string, string[]>();
    const currentLibraryId = mode === "existing" ? activeLibrary?.id ?? null : null;

    rootValidationLibraries.forEach((library) => {
      if (library.id === currentLibraryId) {
        return;
      }

      library.roots.forEach((root) => {
        const normalizedPath = normalizeComparableRootPath(root.path);
        if (!normalizedPath) {
          return;
        }

        const existingNames = otherLibrariesByRootPath.get(normalizedPath);
        if (existingNames) {
          if (!existingNames.includes(library.name)) {
            existingNames.push(library.name);
          }
          return;
        }

        otherLibrariesByRootPath.set(normalizedPath, [library.name]);
      });
    });

    const conflicts = new Map<string, string[]>();
    normalizedDraftRoots.forEach((root) => {
      const normalizedPath = normalizeComparableRootPath(root.path);
      const libraryNames = otherLibrariesByRootPath.get(normalizedPath);
      if (libraryNames?.length) {
        conflicts.set(root.path, libraryNames);
      }
    });

    return conflicts;
  }, [activeLibrary?.id, mode, normalizedDraftRoots, rootValidationLibraries]);
  const sortedFolders = React.useMemo(
    () =>
      normalizedDraftRoots
        .map((rf, i) => ({ rf, originalIndex: i }))
        .sort((a, b) => (a.rf.isDefault === b.rf.isDefault ? 0 : a.rf.isDefault ? -1 : 1)),
    [normalizedDraftRoots],
  );
  const hasRootFolderConflicts = conflictingLibraryNamesByRootPath.size > 0;
  const invalidRootFolderPaths = React.useMemo(() => {
    const invalidPaths = new Set<string>();
    normalizedDraftRoots.forEach((root) => {
      if (!isLocalPathFormatValidForStyle(root.path, localPathStyle)) {
        invalidPaths.add(root.path);
      }
    });
    return invalidPaths;
  }, [localPathStyle, normalizedDraftRoots]);
  const hasInvalidRootFolderPaths = invalidRootFolderPaths.size > 0;
  const actionBusy = loading || librariesLoading || rootValidationLibrariesLoading || saving;
  const settingsBusy = actionBusy || settingsLoading;
  const downloadClientRoutingBusy =
    downloadClientsLoading || draftDownloadClientRoutingLoading;
  const savedRoots = React.useMemo(() => rootsFromLibrary(activeLibrary), [activeLibrary]);
  const draftDownloadClientRoutingEntries = React.useMemo(
    () =>
      draftDownloadClientRoutingMode === "custom"
        ? serializeDownloadClientRoutingEntries(
            downloadClients,
            draftDownloadClientRouting,
            draftDownloadClientRoutingOrder,
          )
        : null,
    [
      downloadClients,
      draftDownloadClientRouting,
      draftDownloadClientRoutingMode,
      draftDownloadClientRoutingOrder,
    ],
  );
  const settingsDraft = React.useMemo<LibrarySettingsDraft>(
    () => ({
      requiredAudioLanguages:
        draftRequiredAudioLanguages.length > 0 ? draftRequiredAudioLanguages : null,
      qualityProfileId:
        draftQualityProfileId === INHERIT_VALUE ? null : draftQualityProfileId,
      requestQualityProfileIds:
        draftRequestQualityProfileIds.length > 0
          ? draftRequestQualityProfileIds
          : null,
      scoringPersona:
        draftScoringPersona === INHERIT_VALUE
          ? null
          : (draftScoringPersona as ScoringPersonaId),
      fillerPolicy:
        isAnimeFacet && draftFillerPolicy !== INHERIT_VALUE ? draftFillerPolicy : null,
      recapPolicy:
        isAnimeFacet && draftRecapPolicy !== INHERIT_VALUE ? draftRecapPolicy : null,
      monitorSpecials:
        isAnimeFacet ? booleanOverrideFromSelectValue(draftMonitorSpecials) : null,
      interSeasonMovies:
        isAnimeFacet ? booleanOverrideFromSelectValue(draftInterSeasonMovies) : null,
      monitorFillerMovies:
        isAnimeFacet ? booleanOverrideFromSelectValue(draftMonitorFillerMovies) : null,
      nfoWriteOnImport: booleanOverrideFromSelectValue(draftNfoWriteOnImport),
      plexmatchWriteOnImport: showPlexmatch
        ? booleanOverrideFromSelectValue(draftPlexmatchWriteOnImport)
        : null,
      importMode:
        draftImportMode === INHERIT_VALUE ? null : (draftImportMode as ImportMode),
      indexerRouting: savedSettings?.indexerRoutingOverride ?? null,
      downloadClientRouting: draftDownloadClientRoutingEntries,
    }),
    [
      draftDownloadClientRoutingEntries,
      draftFillerPolicy,
      draftImportMode,
      draftInterSeasonMovies,
      draftMonitorFillerMovies,
      draftMonitorSpecials,
      draftNfoWriteOnImport,
      draftPlexmatchWriteOnImport,
      draftQualityProfileId,
      draftRequestQualityProfileIds,
      draftRecapPolicy,
      draftRequiredAudioLanguages,
      draftScoringPersona,
      isAnimeFacet,
      savedSettings,
      showPlexmatch,
    ],
  );
  const hasSettingsChanges =
    mode === "new" ||
    (savedSettings !== null &&
      (draftRequiredAudioLanguages.join("\n") !==
        (savedSettings.requiredAudioLanguagesOverride ?? []).join("\n") ||
        settingsDraft.qualityProfileId !== savedSettings.qualityProfileIdOverride ||
        (settingsDraft.requestQualityProfileIds ?? []).join("\n") !==
          (savedSettings.requestQualityProfileIdsOverride ?? []).join("\n") ||
        settingsDraft.scoringPersona !== savedSettings.scoringPersonaOverride ||
        settingsDraft.fillerPolicy !== savedSettings.fillerPolicyOverride ||
        settingsDraft.recapPolicy !== savedSettings.recapPolicyOverride ||
        settingsDraft.monitorSpecials !== savedSettings.monitorSpecialsOverride ||
      settingsDraft.interSeasonMovies !== savedSettings.interSeasonMoviesOverride ||
        settingsDraft.monitorFillerMovies !==
          savedSettings.monitorFillerMoviesOverride ||
        settingsDraft.nfoWriteOnImport !== savedSettings.nfoWriteOnImportOverride ||
        settingsDraft.plexmatchWriteOnImport !==
          savedSettings.plexmatchWriteOnImportOverride ||
        settingsDraft.importMode !== savedSettings.importModeOverride ||
        (draftDownloadClientRoutingMode === "custom") !==
          Boolean(savedDownloadClientRoutingEntries) ||
        (draftDownloadClientRoutingMode === "custom" &&
          (!areNzbgetRoutingMapsEqual(
            draftDownloadClientRouting,
            savedDownloadClientRoutingState.routing,
          ) ||
            !areRoutingOrdersEqual(
              draftDownloadClientRoutingOrder,
              savedDownloadClientRoutingState.order,
            )))));
  const hasDraftChanges =
    mode === "new" ||
    draftName.trim() !== (activeLibrary?.name ?? "") ||
    !rootsEqual(draftRoots, savedRoots) ||
    hasSettingsChanges;
  const selectedValue = mode === "new" ? NEW_LIBRARY_VALUE : activeLibraryId ?? "";

  const handleSelectLibrary = (value: string) => {
    if (value === NEW_LIBRARY_VALUE) {
      setMode("new");
      setActiveLibraryId(null);
      setDraftName("");
      setDraftRoots([]);
      setSavedSettings(null);
      setDraftRequiredAudioLanguages([]);
      setDraftQualityProfileId(INHERIT_VALUE);
      setDraftRequestQualityProfileIds([]);
      setDraftScoringPersona(INHERIT_VALUE);
      setDraftFillerPolicy(INHERIT_VALUE);
      setDraftRecapPolicy(INHERIT_VALUE);
      setDraftMonitorSpecials(INHERIT_VALUE);
      setDraftInterSeasonMovies(INHERIT_VALUE);
      setDraftMonitorFillerMovies(INHERIT_VALUE);
      setDraftNfoWriteOnImport(INHERIT_VALUE);
      setDraftPlexmatchWriteOnImport(INHERIT_VALUE);
      setDraftImportMode(INHERIT_VALUE);
      setDraftDownloadClientRoutingMode("inherit");
      setDraftDownloadClientRouting({});
      setDraftDownloadClientRoutingOrder([]);
      setDraftDownloadClientRoutingLoading(false);
      return;
    }
    setMode("existing");
    setActiveLibraryId(value);
  };

  const handleAddPath = (path: string) => {
    const trimmed = path.trim();
    if (!trimmed) return;
    setDraftRoots((current) => {
      if (current.some((rf) => rf.path === trimmed)) {
        return current;
      }
      return normalizeRoots([...current, { path: trimmed, isDefault: current.length === 0 }]);
    });
  };

  const handleEditPath = (index: number, path: string) => {
    const trimmed = path.trim();
    if (!trimmed) return;
    setDraftRoots((current) => {
      if (current.some((rf, i) => rf.path === trimmed && i !== index)) {
        return current;
      }
      return normalizeRoots(
        current.map((rf, i) => (i === index ? { ...rf, path: trimmed } : rf)),
      );
    });
  };

  const handleRemovePath = (index: number) => {
    setDraftRoots((current) => normalizeRoots(current.filter((_, i) => i !== index)));
  };

  const handleSetDefault = (index: number) => {
    setDraftRoots((current) =>
      normalizeRoots(current.map((rf, i) => ({ ...rf, isDefault: i === index }))),
    );
  };

  const openAdd = () => {
    setEditingIndex(null);
    setBrowserOpen(true);
  };

  const openEdit = (index: number) => {
    setEditingIndex(index);
    setBrowserOpen(true);
  };

  const handleBrowserSelect = (path: string) => {
    if (editingIndex !== null) {
      handleEditPath(editingIndex, path);
    } else {
      handleAddPath(path);
    }
  };

  const handleNewLibrary = () => {
    setMode("new");
    setActiveLibraryId(null);
    setDraftName("");
    setDraftRoots([]);
    setSavedSettings(null);
    setDraftRequiredAudioLanguages([]);
    setDraftQualityProfileId(INHERIT_VALUE);
    setDraftRequestQualityProfileIds([]);
    setDraftScoringPersona(INHERIT_VALUE);
    setDraftFillerPolicy(INHERIT_VALUE);
    setDraftRecapPolicy(INHERIT_VALUE);
    setDraftMonitorSpecials(INHERIT_VALUE);
    setDraftInterSeasonMovies(INHERIT_VALUE);
    setDraftMonitorFillerMovies(INHERIT_VALUE);
    setDraftNfoWriteOnImport(INHERIT_VALUE);
    setDraftPlexmatchWriteOnImport(INHERIT_VALUE);
    setDraftImportMode(INHERIT_VALUE);
    setDraftDownloadClientRoutingMode("inherit");
    setDraftDownloadClientRouting({});
    setDraftDownloadClientRoutingOrder([]);
    setDraftDownloadClientRoutingLoading(false);
  };

  const handleDownloadClientRoutingModeChange = React.useCallback(
    async (nextMode: "inherit" | "custom") => {
      if (nextMode === "inherit") {
        setDraftDownloadClientRoutingMode("inherit");
        return;
      }

      if (draftDownloadClientRoutingMode === "custom") {
        setDraftDownloadClientRoutingMode("custom");
        return;
      }

      if (savedDownloadClientRoutingEntries) {
        setDraftDownloadClientRoutingMode("custom");
        return;
      }

      setSettingsError(null);
      setDraftDownloadClientRoutingLoading(true);
      try {
        const entries = await loadFacetDownloadClientRouting(currentFacet);
        const nextState = buildDownloadClientRoutingState(downloadClients, entries);
        setDraftDownloadClientRoutingMode("custom");
        setDraftDownloadClientRouting(nextState.routing);
        setDraftDownloadClientRoutingOrder(nextState.order);
      } catch (error) {
        setSettingsError(
          error instanceof Error ? error.message : t("status.failedToLoad"),
        );
      } finally {
        setDraftDownloadClientRoutingLoading(false);
      }
    },
    [
      currentFacet,
      downloadClients,
      draftDownloadClientRoutingMode,
      loadFacetDownloadClientRouting,
      savedDownloadClientRoutingEntries,
      t,
    ],
  );

  const updateDownloadClientRoutingDraft = React.useCallback(
    (
      clientId: string,
      nextValue: Partial<{
        enabled: boolean;
        category: string;
        recentQueuePriority: string;
        olderQueuePriority: string;
        removeCompleted: boolean;
        removeFailed: boolean;
      }>,
    ) => {
      setDraftDownloadClientRouting((current) => ({
        ...current,
        [clientId]: {
          ...(current[clientId] ?? disabledDownloadClientRoutingSettings()),
          ...nextValue,
        },
      }));
      setDraftDownloadClientRoutingOrder((current) =>
        current.includes(clientId) ? current : [...current, clientId],
      );
    },
    [],
  );

  const moveDownloadClientRoutingDraft = React.useCallback(
    (clientId: string, direction: "up" | "down") => {
      setDraftDownloadClientRoutingOrder((current) => {
        const index = current.indexOf(clientId);
        if (index < 0) {
          return current;
        }

        const nextIndex = direction === "up" ? index - 1 : index + 1;
        if (nextIndex < 0 || nextIndex >= current.length) {
          return current;
        }

        const next = [...current];
        [next[index], next[nextIndex]] = [next[nextIndex], next[index]];
        return next;
      });
    },
    [],
  );

  const handleSaveLibrary = async () => {
    if (downloadClientRoutingBusy) {
      return null;
    }
    const name = draftName.trim();
    if (!name) {
      return null;
    }
    const roots = normalizeRoots(draftRoots);
    setDraftRoots(roots);
    if (mode === "new") {
      const created = await onCreateLibrary({ name, roots, settings: settingsDraft });
      if (created?.id) {
        try {
          const refreshedSettings = await loadLibrarySettings(created.id);
          hydrateSavedSettings(refreshedSettings);
          setSettingsError(null);
        } catch (error) {
          setSettingsError(
            error instanceof Error ? error.message : t("settings.librarySettingsLoadFailed"),
          );
        }
        setMode("existing");
        setActiveLibraryId(created.id);
      }
      return created ?? null;
    }
    if (activeLibrary) {
      const updatedLibrary =
        (await onUpdateLibrary(activeLibrary.id, {
          name,
          roots,
          settings: settingsDraft,
        })) ?? activeLibrary;
      try {
        const refreshedSettings = await loadLibrarySettings(updatedLibrary.id);
        hydrateSavedSettings(refreshedSettings);
        setSettingsError(null);
      } catch (error) {
        setSettingsError(
          error instanceof Error ? error.message : t("settings.librarySettingsLoadFailed"),
        );
      }
      return updatedLibrary;
    }
    return null;
  };

  const handleSaveAndScanLibrary = async () => {
    const savedLibrary = await handleSaveLibrary();
    const libraryId = savedLibrary?.id ?? (mode === "existing" ? activeLibrary?.id : null);
    if (!libraryId) {
      return;
    }
    void onScan(libraryId);
  };

  const handleDeleteLibrary = async () => {
    if (!activeLibrary || activeLibrary.isDefault) {
      return;
    }
    if (!window.confirm(t("settings.libraryDeleteConfirm", { name: activeLibrary.name }))) {
      return;
    }
    await onDeleteLibrary(activeLibrary.id);
  };

  const handleScan = () => {
    if (!activeLibrary || mode === "new") {
      return;
    }
    void onScan(activeLibrary.id);
  };

  const browserInitialPath = editingIndex !== null
    ? normalizedDraftRoots[editingIndex]?.path ?? "/"
    : "/";

  const browserTitle = editingIndex !== null
    ? t("settings.rootFolderEdit")
    : t("settings.rootFolderAdd");
  const libraryScanDisabled =
    scanLoading || actionBusy || mode === "new" || !activeLibrary;
  const libraryScanSummaryText = scanSummary
    ? t("settings.libraryScanSummary", {
        imported: scanSummary.imported,
        skipped: scanSummary.skipped,
        unmatched: scanSummary.unmatched,
      })
    : null;

  return (
    <>
      <Card id="media-library-settings-panel">
        <CardHeader className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
          <CardTitle>{settingsTitle}</CardTitle>
          <Button
            type="button"
            variant="default"
            onClick={handleScan}
            disabled={libraryScanDisabled}
          >
            <RefreshCw className={`mr-1.5 h-4 w-4${scanLoading ? " animate-spin" : ""}`} />
            {scanLoading
              ? t("settings.libraryScanRunning")
              : t("settings.libraryScanButton")}
          </Button>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
            <Select
              value={selectedValue}
              onValueChange={handleSelectLibrary}
              disabled={actionBusy || libraries.length === 0}
            >
              <SelectTrigger id="media-library-select" className="w-full sm:w-[260px]">
                <SelectValue placeholder={t("settings.librariesLabel")} />
              </SelectTrigger>
              <SelectContent>
                {mode === "new" ? (
                  <SelectItem value={NEW_LIBRARY_VALUE}>{t("settings.libraryNew")}</SelectItem>
                ) : null}
                {libraries.map((library) => (
                  <SelectItem key={library.id} value={library.id}>
                    {library.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <Button
              id="media-library-new"
              type="button"
              variant="outline"
              onClick={handleNewLibrary}
              disabled={actionBusy}
            >
              <Plus className="mr-1.5 h-4 w-4" />
              {t("settings.libraryNewButton")}
            </Button>
          </div>

          {scanSummary ? (
            <p className="text-xs text-muted-foreground">{libraryScanSummaryText}</p>
          ) : null}
          {scanNotice ? (
            <p className="text-xs text-destructive">{scanNotice}</p>
          ) : null}

          {libraries.length === 0 && !librariesLoading && mode !== "new" ? (
            <p id="media-library-empty" className="text-sm text-muted-foreground">
              {t("settings.libraryEmpty")}
            </p>
          ) : null}

          {mode === "new" || activeLibrary ? (
            <div className="space-y-4">
              <div className="grid gap-3 md:grid-cols-[minmax(0,1fr)_minmax(12rem,16rem)]">
                <div className="space-y-2">
                  <Label htmlFor="media-library-name">{t("settings.libraryNameLabel")}</Label>
                  <Input
                    id="media-library-name"
                    value={draftName}
                    onChange={(event) => setDraftName(event.target.value)}
                    placeholder={t("settings.libraryNamePlaceholder")}
                    disabled={actionBusy}
                  />
                </div>
              </div>

              <div className="space-y-3">
                <Label className="block">{t("settings.rootFoldersLabel")}</Label>
                {normalizedDraftRoots.length === 0 && !loading ? (
                  <p className="text-xs text-muted-foreground">{t("settings.rootFoldersEmpty")}</p>
                ) : null}
                <ul className="space-y-2">
                  {sortedFolders.map(({ rf, originalIndex: index }) => {
                    const conflictingLibraryNames =
                      conflictingLibraryNamesByRootPath.get(rf.path) ?? null;
                    const pathIsInvalid = invalidRootFolderPaths.has(rf.path);

                    return (
                      <li
                        key={`${rf.path}-${index}`}
                        id={selectorId("media-library-root-row", rf.path)}
                        className="space-y-1"
                      >
                        <div className="flex items-center gap-2">
                          <code className="flex-1 truncate rounded-md border border-border bg-muted/50 px-3 py-1.5 font-mono text-sm">
                            {rf.path}
                          </code>
                          {rf.isDefault ? (
                            <span className="shrink-0 rounded-md bg-muted px-2 py-1 text-xs text-muted-foreground">
                              {t("label.default")}
                            </span>
                          ) : (
                            <button
                              id={selectorId("media-library-root-set-default", rf.path)}
                              type="button"
                              className="shrink-0 rounded-md px-2 py-1 text-xs text-muted-foreground hover:text-foreground hover:underline"
                              onClick={() => handleSetDefault(index)}
                              disabled={actionBusy}
                            >
                              {t("settings.rootFolderSetDefault")}
                            </button>
                          )}
                          <Button
                            id={selectorId("media-library-root-edit", rf.path)}
                            type="button"
                            variant="secondary"
                            size="icon-sm"
                            className={cn(
                              boxedActionButtonBaseClass,
                              boxedActionButtonToneClass.edit,
                            )}
                            onClick={() => openEdit(index)}
                            disabled={actionBusy}
                            aria-label={t("label.edit")}
                            title={t("label.edit")}
                          >
                            <Pencil className="h-4 w-4" />
                          </Button>
                          <Button
                            id={selectorId("media-library-root-delete", rf.path)}
                            type="button"
                            variant="secondary"
                            size="icon-sm"
                            className={cn(
                              boxedActionButtonBaseClass,
                              boxedActionButtonToneClass.delete,
                            )}
                            onClick={() => handleRemovePath(index)}
                            disabled={actionBusy}
                            aria-label={t("label.delete")}
                            title={t("label.delete")}
                          >
                            <Trash2 className="h-4 w-4" />
                          </Button>
                        </div>
                        {conflictingLibraryNames ? (
                          <p className="text-xs text-destructive">
                            {t("settings.rootFolderConflict", {
                              libraries: conflictingLibraryNames.join(", "),
                            })}
                          </p>
                        ) : null}
                        {pathIsInvalid ? (
                          <p className="text-xs text-destructive">
                            {t("settings.downloadClientRemotePathMappingsLocalRequired")}
                          </p>
                        ) : null}
                      </li>
                    );
                  })}
                </ul>
                <Button
                  id="media-library-add-root"
                  type="button"
                  variant="outline"
                  onClick={openAdd}
                  disabled={actionBusy}
                >
                  <FolderOpen className="mr-1.5 h-4 w-4" />
                  {t("settings.rootFolderAdd")}
                </Button>
                <p className="text-xs text-muted-foreground">
                  {loading ? t("label.loading") : t("settings.rootFoldersHelp")}
                </p>
              </div>

              <div className="grid gap-3 md:grid-cols-3">
                <div className="space-y-2">
                  <Label>{t("settings.libraryRequiredAudioLabel")}</Label>
                  <SubtitleLanguagePicker
                    value={draftRequiredAudioLanguages}
                    onChange={setDraftRequiredAudioLanguages}
                    disabled={settingsBusy}
                  />
                  {savedSettings ? (
                    <p className="text-xs text-muted-foreground">
                      {t("settings.libraryEffectiveAudio", {
                        value: savedSettings.requiredAudioLanguages.join(", ") || t("label.none"),
                      })}
                    </p>
                  ) : null}
                  <div className="space-y-2 rounded-md border border-border/70 p-2">
                    <Label>{t("settings.libraryRequestQualityProfilesLabel")}</Label>
                    {qualityProfiles.map((profile) => {
                      const checked = draftRequestQualityProfileIds.includes(profile.id);
                      return (
                        <label
                          key={profile.id}
                          className="flex items-center gap-2 text-xs text-muted-foreground"
                        >
                          <Checkbox
                            checked={checked}
                            disabled={settingsBusy}
                            onCheckedChange={(value) => {
                              setDraftRequestQualityProfileIds((current) =>
                                value
                                  ? [...current, profile.id]
                                  : current.filter((profileId) => profileId !== profile.id),
                              );
                            }}
                          />
                          <span>{profile.name}</span>
                        </label>
                      );
                    })}
                    <p className="text-xs text-muted-foreground">
                      {t("settings.libraryRequestQualityProfilesHelp")}
                    </p>
                  </div>
                </div>
                <div className="space-y-2">
                  <Label>{t("settings.libraryQualityProfileLabel")}</Label>
                  <Select
                    value={draftQualityProfileId}
                    onValueChange={setDraftQualityProfileId}
                    disabled={settingsBusy}
                  >
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value={INHERIT_VALUE}>
                        {t("settings.libraryInheritFacet")}
                      </SelectItem>
                      {qualityProfiles.map((profile) => (
                        <SelectItem key={profile.id} value={profile.id}>
                          {profile.name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  {savedSettings ? (
                    <p className="text-xs text-muted-foreground">
                      {t("settings.libraryEffectiveProfile", {
                        value:
                          qualityProfiles.find(
                            (profile) => profile.id === savedSettings.qualityProfileId,
                          )?.name ?? savedSettings.qualityProfileId,
                      })}
                    </p>
                  ) : null}
                </div>
                <div className="space-y-2">
                  <Label>{t("settings.libraryScoringPersonaLabel")}</Label>
                  <Select
                    value={draftScoringPersona}
                    onValueChange={setDraftScoringPersona}
                    disabled={settingsBusy}
                  >
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value={INHERIT_VALUE}>
                        {t("settings.libraryInheritFacet")}
                      </SelectItem>
                      {SCORING_PERSONA_CHOICES.map((choice) => (
                        <SelectItem key={choice.value} value={choice.value}>
                          {t(choice.labelKey)}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  {savedSettings ? (
                    <p className="text-xs text-muted-foreground">
                      {t("settings.libraryEffectivePersona", {
                        value: t(
                          SCORING_PERSONA_CHOICES.find(
                            (choice) => choice.value === savedSettings.scoringPersona,
                          )?.labelKey ?? "qualityProfile.personaBalanced",
                        ),
                      })}
                    </p>
                  ) : null}
                </div>
              </div>

              <div className="rounded-lg border border-border/70 bg-muted/10 p-4">
                <div className="max-w-md space-y-2">
                  <Label>{t("settings.importModeLabel")}</Label>
                  <Select
                    value={draftImportMode}
                    onValueChange={setDraftImportMode}
                    disabled={settingsBusy}
                  >
                    <SelectTrigger id="media-library-import-mode-trigger">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {IMPORT_MODE_OPTIONS.map((option) => (
                        <SelectItem
                          id={selectorId("media-library-import-mode-option", option.value)}
                          key={option.value}
                          value={option.value}
                        >
                          {t(option.labelKey)}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <p className="text-xs text-muted-foreground">
                    {t("settings.importModeDescription")}
                  </p>
                  {savedSettings ? (
                    <p className="text-xs text-muted-foreground">
                      {t("settings.libraryEffectiveProfile", {
                        value: t(importModeLabelKey(savedSettings.importMode)),
                      })}
                    </p>
                  ) : null}
                </div>
              </div>

              <div className="space-y-3">
                <div className="space-y-2">
                  <Label>{t("settings.downloadClientRouting")}</Label>
                  <Select
                    value={draftDownloadClientRoutingMode}
                    onValueChange={(value) => {
                      void handleDownloadClientRoutingModeChange(
                        value as "inherit" | "custom",
                      );
                    }}
                    disabled={settingsBusy || downloadClientRoutingBusy}
                  >
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="inherit">
                        {t("settings.libraryInheritFacet")}
                      </SelectItem>
                      <SelectItem value="custom">
                        {t("settings.libraryCustomRouting")}
                      </SelectItem>
                    </SelectContent>
                  </Select>
                </div>
                {draftDownloadClientRoutingMode === "custom" ? (
                  <DownloadClientRoutingPanel
                    scopeLabel={activeLibrary?.name ?? settingsTitle}
                    downloadClients={downloadClients}
                    activeScopeRouting={draftDownloadClientRouting}
                    activeScopeRoutingOrder={draftDownloadClientRoutingOrder}
                    downloadClientRoutingLoading={downloadClientRoutingBusy}
                    downloadClientRoutingSaving={saving}
                    updateDownloadClientRoutingForScope={
                      updateDownloadClientRoutingDraft
                    }
                    moveDownloadClientInScope={moveDownloadClientRoutingDraft}
                  />
                ) : null}
              </div>

              <div className="rounded-lg border border-border/70 bg-muted/10 p-4">
                <div className="space-y-1">
                  <h3 className="text-sm font-medium text-card-foreground">
                    {t("settings.sidecarFilesTitle")}
                  </h3>
                </div>
                <div
                  className={`mt-4 grid gap-3 ${showPlexmatch ? "md:grid-cols-2" : "md:grid-cols-1"}`}
                >
                  <div className="space-y-2">
                    <Label>{t("settings.nfoWriteOnImportLabel")}</Label>
                    <Select
                      value={draftNfoWriteOnImport}
                      onValueChange={setDraftNfoWriteOnImport}
                      disabled={settingsBusy}
                    >
                      <SelectTrigger>
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {BOOLEAN_OVERRIDE_OPTIONS.map((option) => (
                          <SelectItem key={option.value} value={option.value}>
                            {t(option.labelKey)}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    <p className="text-xs text-muted-foreground">
                      {t("settings.nfoWriteOnImportDescription")}
                    </p>
                    {savedSettings ? (
                      <p className="text-xs text-muted-foreground">
                        {t("settings.libraryEffectiveProfile", {
                          value: t(
                            savedSettings.nfoWriteOnImport
                              ? "label.enabled"
                              : "label.disabled",
                          ),
                        })}
                      </p>
                    ) : null}
                  </div>
                  {showPlexmatch ? (
                    <div className="space-y-2">
                      <Label>{t("settings.plexmatchWriteOnImportLabel")}</Label>
                      <Select
                        value={draftPlexmatchWriteOnImport}
                        onValueChange={setDraftPlexmatchWriteOnImport}
                        disabled={settingsBusy}
                      >
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          {BOOLEAN_OVERRIDE_OPTIONS.map((option) => (
                            <SelectItem key={option.value} value={option.value}>
                              {t(option.labelKey)}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                      <p className="text-xs text-muted-foreground">
                        {t("settings.plexmatchWriteOnImportDescription")}
                      </p>
                      {savedSettings?.plexmatchWriteOnImport != null ? (
                        <p className="text-xs text-muted-foreground">
                          {t("settings.libraryEffectiveProfile", {
                            value: t(
                              savedSettings.plexmatchWriteOnImport
                                ? "label.enabled"
                                : "label.disabled",
                            ),
                          })}
                        </p>
                      ) : null}
                    </div>
                  ) : null}
                </div>
              </div>

              {isAnimeFacet ? (
                <div className="rounded-lg border border-border/70 bg-muted/10 p-4">
                  <div className="space-y-1">
                    <h3 className="text-sm font-medium text-card-foreground">
                      {t("settings.animeSettings")}
                    </h3>
                  </div>
                  <div className="mt-4 grid gap-3 md:grid-cols-2 xl:grid-cols-3">
                    <div className="space-y-2">
                      <Label>{t("settings.fillerPolicyLabel")}</Label>
                      <Select
                        value={draftFillerPolicy}
                        onValueChange={setDraftFillerPolicy}
                        disabled={settingsBusy}
                      >
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value={INHERIT_VALUE}>
                            {t("settings.libraryInheritFacet")}
                          </SelectItem>
                          {FILLER_POLICY_OPTIONS.map((option) => (
                            <SelectItem key={option.value} value={option.value}>
                              {t(option.labelKey)}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                      {savedSettings?.fillerPolicy ? (
                        <p className="text-xs text-muted-foreground">
                          {t("settings.libraryEffectiveProfile", {
                            value: t(fillerPolicyLabelKey(savedSettings.fillerPolicy)),
                          })}
                        </p>
                      ) : null}
                    </div>
                    <div className="space-y-2">
                      <Label>{t("settings.recapPolicyLabel")}</Label>
                      <Select
                        value={draftRecapPolicy}
                        onValueChange={setDraftRecapPolicy}
                        disabled={settingsBusy}
                      >
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value={INHERIT_VALUE}>
                            {t("settings.libraryInheritFacet")}
                          </SelectItem>
                          {RECAP_POLICY_OPTIONS.map((option) => (
                            <SelectItem key={option.value} value={option.value}>
                              {t(option.labelKey)}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                      {savedSettings?.recapPolicy ? (
                        <p className="text-xs text-muted-foreground">
                          {t("settings.libraryEffectiveProfile", {
                            value: t(recapPolicyLabelKey(savedSettings.recapPolicy)),
                          })}
                        </p>
                      ) : null}
                    </div>
                    <div className="space-y-2">
                      <Label>{t("settings.monitorSpecialsLabel")}</Label>
                      <Select
                        value={draftMonitorSpecials}
                        onValueChange={setDraftMonitorSpecials}
                        disabled={settingsBusy}
                      >
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          {BOOLEAN_OVERRIDE_OPTIONS.map((option) => (
                            <SelectItem key={option.value} value={option.value}>
                              {t(option.labelKey)}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                      {savedSettings?.monitorSpecials != null ? (
                        <p className="text-xs text-muted-foreground">
                          {t("settings.libraryEffectiveProfile", {
                            value: t(
                              savedSettings.monitorSpecials
                                ? "label.enabled"
                                : "label.disabled",
                            ),
                          })}
                        </p>
                      ) : null}
                    </div>
                    <div className="space-y-2">
                      <Label>{t("settings.interSeasonMoviesLabel")}</Label>
                      <Select
                        value={draftInterSeasonMovies}
                        onValueChange={setDraftInterSeasonMovies}
                        disabled={settingsBusy}
                      >
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          {BOOLEAN_OVERRIDE_OPTIONS.map((option) => (
                            <SelectItem key={option.value} value={option.value}>
                              {t(option.labelKey)}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                      {savedSettings?.interSeasonMovies != null ? (
                        <p className="text-xs text-muted-foreground">
                          {t("settings.libraryEffectiveProfile", {
                            value: t(
                              savedSettings.interSeasonMovies
                                ? "label.enabled"
                                : "label.disabled",
                            ),
                          })}
                        </p>
                      ) : null}
                    </div>
                    <div className="space-y-2">
                      <Label>{t("settings.monitorFillerMoviesLabel")}</Label>
                      <Select
                        value={draftMonitorFillerMovies}
                        onValueChange={setDraftMonitorFillerMovies}
                        disabled={settingsBusy}
                      >
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          {BOOLEAN_OVERRIDE_OPTIONS.map((option) => (
                            <SelectItem key={option.value} value={option.value}>
                              {t(option.labelKey)}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                      {savedSettings?.monitorFillerMovies != null ? (
                        <p className="text-xs text-muted-foreground">
                          {t("settings.libraryEffectiveProfile", {
                            value: t(
                              savedSettings.monitorFillerMovies
                                ? "label.enabled"
                                : "label.disabled",
                            ),
                          })}
                        </p>
                      ) : null}
                    </div>
                  </div>
                </div>
              ) : null}
              {settingsError ? (
                <p className="text-xs text-destructive">{settingsError}</p>
              ) : null}

              <div className="flex flex-wrap items-center gap-2">
                <Button
                  id="media-library-save-scan"
                  type="button"
                  variant="primary"
                  onClick={handleSaveAndScanLibrary}
                  disabled={
                    settingsBusy ||
                    downloadClientRoutingBusy ||
                    !draftName.trim() ||
                    !hasDraftChanges ||
                    hasRootFolderConflicts ||
                    hasInvalidRootFolderPaths
                  }
                >
                  <Save className="mr-1.5 h-4 w-4" />
                  {t("settings.librarySaveAndScanButton")}
                </Button>
                <Button
                  id="media-library-save"
                  type="button"
                  variant="outline"
                  onClick={handleSaveLibrary}
                  disabled={
                    settingsBusy ||
                    downloadClientRoutingBusy ||
                    !draftName.trim() ||
                    !hasDraftChanges ||
                    hasRootFolderConflicts ||
                    hasInvalidRootFolderPaths
                  }
                >
                  {t("settings.librarySaveOnlyButton")}
                </Button>
                {mode !== "new" && activeLibrary && !activeLibrary.isDefault ? (
                  <Button
                    type="button"
                    variant="outline"
                    onClick={handleDeleteLibrary}
                    disabled={actionBusy}
                    className={boxedActionButtonToneClass.delete}
                  >
                    <Trash2 className="mr-1.5 h-4 w-4" />
                    {t("settings.libraryDeleteButton")}
                  </Button>
                ) : null}
              </div>
            </div>
          ) : null}
        </CardContent>
      </Card>
      <FolderBrowserDialog
        open={browserOpen}
        onOpenChange={setBrowserOpen}
        onSelect={handleBrowserSelect}
        initialPath={browserInitialPath}
        title={browserTitle}
      />
    </>
  );
});
