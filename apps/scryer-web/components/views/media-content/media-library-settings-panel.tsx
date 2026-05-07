import * as React from "react";
import { FolderOpen, Pencil, Plus, RefreshCw, Save, Trash2 } from "lucide-react";
import { SubtitleLanguagePicker } from "@/components/common/subtitle-language-picker";
import { FolderBrowserDialog } from "@/components/setup/folder-browser-dialog";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
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
import { boxedActionButtonToneClass } from "@/lib/utils/action-button-styles";
import type {
  LibraryRecord,
  LibraryScanSummary,
  LibrarySettingsDraft,
  LibrarySettingsRecord,
  ParsedQualityProfile,
  RootFolderOption,
  ScoringPersonaId,
} from "@/lib/types";

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
  qualityProfiles: ParsedQualityProfile[];
  loadLibrarySettings: (libraryId: string) => Promise<LibrarySettingsRecord | null>;
  onCreateLibrary: (input: LibraryMutationInput) => Promise<LibraryRecord | null | void> | LibraryRecord | null | void;
  onUpdateLibrary: (libraryId: string, input: LibraryMutationInput) => Promise<LibraryRecord | null | void> | LibraryRecord | null | void;
  onDeleteLibrary: (libraryId: string) => Promise<boolean | void> | boolean | void;
  onScan: (libraryId: string) => Promise<void> | void;
};

const NEW_LIBRARY_VALUE = "__new_library__";

function rootsFromLibrary(library: LibraryRecord | null): RootFolderOption[] {
  return (library?.roots ?? []).map((root) => ({
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
    next.push({ path, isDefault });
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
  qualityProfiles,
  loadLibrarySettings,
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
  const [draftScoringPersona, setDraftScoringPersona] = React.useState(INHERIT_VALUE);
  const [draftFillerPolicy, setDraftFillerPolicy] = React.useState(INHERIT_VALUE);
  const [draftRecapPolicy, setDraftRecapPolicy] = React.useState(INHERIT_VALUE);
  const [draftMonitorSpecials, setDraftMonitorSpecials] = React.useState(INHERIT_VALUE);
  const [draftInterSeasonMovies, setDraftInterSeasonMovies] = React.useState(INHERIT_VALUE);
  const [draftMonitorFillerMovies, setDraftMonitorFillerMovies] = React.useState(INHERIT_VALUE);
  const [draftNfoWriteOnImport, setDraftNfoWriteOnImport] = React.useState(INHERIT_VALUE);
  const [draftPlexmatchWriteOnImport, setDraftPlexmatchWriteOnImport] = React.useState(INHERIT_VALUE);
  const [savedSettings, setSavedSettings] = React.useState<LibrarySettingsRecord | null>(null);
  const [browserOpen, setBrowserOpen] = React.useState(false);
  const [editingIndex, setEditingIndex] = React.useState<number | null>(null);

  const activeLibrary = React.useMemo(
    () => libraries.find((library) => library.id === activeLibraryId) ?? null,
    [activeLibraryId, libraries],
  );
  const currentFacet = activeLibrary?.facet ?? facet;
  const isAnimeFacet = currentFacet === "anime";
  const showPlexmatch = currentFacet === "series" || currentFacet === "anime";

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
      setDraftScoringPersona(INHERIT_VALUE);
      setDraftFillerPolicy(INHERIT_VALUE);
      setDraftRecapPolicy(INHERIT_VALUE);
      setDraftMonitorSpecials(INHERIT_VALUE);
      setDraftInterSeasonMovies(INHERIT_VALUE);
      setDraftMonitorFillerMovies(INHERIT_VALUE);
      setDraftNfoWriteOnImport(INHERIT_VALUE);
      setDraftPlexmatchWriteOnImport(INHERIT_VALUE);
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
        setSavedSettings(settings);
        setDraftRequiredAudioLanguages(settings?.requiredAudioLanguagesOverride ?? []);
        setDraftQualityProfileId(settings?.qualityProfileIdOverride ?? INHERIT_VALUE);
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
  }, [activeLibrary, loadLibrarySettings, mode, t]);

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
  const actionBusy = loading || librariesLoading || rootValidationLibrariesLoading || saving;
  const settingsBusy = actionBusy || settingsLoading;
  const savedRoots = React.useMemo(() => rootsFromLibrary(activeLibrary), [activeLibrary]);
  const settingsDraft = React.useMemo<LibrarySettingsDraft>(
    () => ({
      requiredAudioLanguages:
        draftRequiredAudioLanguages.length > 0 ? draftRequiredAudioLanguages : null,
      qualityProfileId:
        draftQualityProfileId === INHERIT_VALUE ? null : draftQualityProfileId,
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
      indexerRouting: savedSettings?.indexerRoutingOverride ?? null,
      downloadClientRouting: savedSettings?.downloadClientRoutingOverride ?? null,
    }),
    [
      draftFillerPolicy,
      draftInterSeasonMovies,
      draftMonitorFillerMovies,
      draftMonitorSpecials,
      draftNfoWriteOnImport,
      draftPlexmatchWriteOnImport,
      draftQualityProfileId,
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
        settingsDraft.scoringPersona !== savedSettings.scoringPersonaOverride ||
        settingsDraft.fillerPolicy !== savedSettings.fillerPolicyOverride ||
        settingsDraft.recapPolicy !== savedSettings.recapPolicyOverride ||
        settingsDraft.monitorSpecials !== savedSettings.monitorSpecialsOverride ||
        settingsDraft.interSeasonMovies !== savedSettings.interSeasonMoviesOverride ||
        settingsDraft.monitorFillerMovies !==
          savedSettings.monitorFillerMoviesOverride ||
        settingsDraft.nfoWriteOnImport !== savedSettings.nfoWriteOnImportOverride ||
        settingsDraft.plexmatchWriteOnImport !==
          savedSettings.plexmatchWriteOnImportOverride));
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
      setDraftScoringPersona(INHERIT_VALUE);
      setDraftFillerPolicy(INHERIT_VALUE);
      setDraftRecapPolicy(INHERIT_VALUE);
      setDraftMonitorSpecials(INHERIT_VALUE);
      setDraftInterSeasonMovies(INHERIT_VALUE);
      setDraftMonitorFillerMovies(INHERIT_VALUE);
      setDraftNfoWriteOnImport(INHERIT_VALUE);
      setDraftPlexmatchWriteOnImport(INHERIT_VALUE);
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
    setDraftScoringPersona(INHERIT_VALUE);
    setDraftFillerPolicy(INHERIT_VALUE);
    setDraftRecapPolicy(INHERIT_VALUE);
    setDraftMonitorSpecials(INHERIT_VALUE);
    setDraftInterSeasonMovies(INHERIT_VALUE);
    setDraftMonitorFillerMovies(INHERIT_VALUE);
    setDraftNfoWriteOnImport(INHERIT_VALUE);
    setDraftPlexmatchWriteOnImport(INHERIT_VALUE);
  };

  const handleSaveLibrary = async () => {
    const name = draftName.trim();
    if (!name) {
      return null;
    }
    const roots = normalizeRoots(draftRoots);
    setDraftRoots(roots);
    if (mode === "new") {
      const created = await onCreateLibrary({ name, roots, settings: settingsDraft });
      if (created?.id) {
        setMode("existing");
        setActiveLibraryId(created.id);
      }
      return created ?? null;
    }
    if (activeLibrary) {
      return (
        (await onUpdateLibrary(activeLibrary.id, {
          name,
          roots,
          settings: settingsDraft,
        })) ?? activeLibrary
      );
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
      <Card>
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
              <SelectTrigger className="w-full sm:w-[260px]">
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
            <p className="text-sm text-muted-foreground">{t("settings.libraryEmpty")}</p>
          ) : null}

          {mode === "new" || activeLibrary ? (
            <div className="space-y-4">
              <div className="grid gap-3 md:grid-cols-[minmax(0,1fr)_minmax(12rem,16rem)]">
                <div className="space-y-2">
                  <Label htmlFor="library-name">{t("settings.libraryNameLabel")}</Label>
                  <Input
                    id="library-name"
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

                    return (
                      <li key={`${rf.path}-${index}`} className="space-y-1">
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
                              type="button"
                              className="shrink-0 rounded-md px-2 py-1 text-xs text-muted-foreground hover:text-foreground hover:underline"
                              onClick={() => handleSetDefault(index)}
                              disabled={actionBusy}
                            >
                              {t("settings.rootFolderSetDefault")}
                            </button>
                          )}
                          <Button
                            type="button"
                            variant="ghost"
                            size="icon"
                            className="h-8 w-8 shrink-0"
                            onClick={() => openEdit(index)}
                            disabled={actionBusy}
                            aria-label={t("label.edit")}
                          >
                            <Pencil className="h-4 w-4" />
                          </Button>
                          <Button
                            type="button"
                            variant="ghost"
                            size="icon"
                            className="h-8 w-8 shrink-0 text-destructive hover:text-destructive"
                            onClick={() => handleRemovePath(index)}
                            disabled={actionBusy}
                            aria-label={t("label.delete")}
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
                      </li>
                    );
                  })}
                </ul>
                <Button
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
                  type="button"
                  variant="primary"
                  onClick={handleSaveAndScanLibrary}
                  disabled={
                    settingsBusy ||
                    !draftName.trim() ||
                    !hasDraftChanges ||
                    hasRootFolderConflicts
                  }
                >
                  <Save className="mr-1.5 h-4 w-4" />
                  {t("settings.librarySaveAndScanButton")}
                </Button>
                <Button
                  type="button"
                  variant="outline"
                  onClick={handleSaveLibrary}
                  disabled={
                    settingsBusy ||
                    !draftName.trim() ||
                    !hasDraftChanges ||
                    hasRootFolderConflicts
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
