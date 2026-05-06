import * as React from "react";
import { FolderOpen, Pencil, Plus, Save, Trash2 } from "lucide-react";
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

type LibraryMutationInput = {
  name: string;
  roots: RootFolderOption[];
  settings?: LibrarySettingsDraft;
};

type MediaLibrarySettingsPanelProps = {
  settingsTitle: string;
  libraries: LibraryRecord[];
  librariesLoading: boolean;
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

export const MediaLibrarySettingsPanel = React.memo(function MediaLibrarySettingsPanel({
  settingsTitle,
  libraries,
  librariesLoading,
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
  const [draftRequiredAudio, setDraftRequiredAudio] = React.useState("");
  const [draftQualityProfileId, setDraftQualityProfileId] = React.useState(INHERIT_VALUE);
  const [draftScoringPersona, setDraftScoringPersona] = React.useState(INHERIT_VALUE);
  const [savedSettings, setSavedSettings] = React.useState<LibrarySettingsRecord | null>(null);
  const [browserOpen, setBrowserOpen] = React.useState(false);
  const [editingIndex, setEditingIndex] = React.useState<number | null>(null);

  const activeLibrary = React.useMemo(
    () => libraries.find((library) => library.id === activeLibraryId) ?? null,
    [activeLibraryId, libraries],
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
      setDraftRequiredAudio("");
      setDraftQualityProfileId(INHERIT_VALUE);
      setDraftScoringPersona(INHERIT_VALUE);
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
        setDraftRequiredAudio((settings?.requiredAudioLanguagesOverride ?? []).join(", "));
        setDraftQualityProfileId(settings?.qualityProfileIdOverride ?? INHERIT_VALUE);
        setDraftScoringPersona(settings?.scoringPersonaOverride ?? INHERIT_VALUE);
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
  const sortedFolders = React.useMemo(
    () =>
      normalizedDraftRoots
        .map((rf, i) => ({ rf, originalIndex: i }))
        .sort((a, b) => (a.rf.isDefault === b.rf.isDefault ? 0 : a.rf.isDefault ? -1 : 1)),
    [normalizedDraftRoots],
  );
  const actionBusy = loading || librariesLoading || saving;
  const settingsBusy = actionBusy || settingsLoading;
  const savedRoots = React.useMemo(() => rootsFromLibrary(activeLibrary), [activeLibrary]);
  const draftRequiredAudioLanguages = React.useMemo(
    () =>
      draftRequiredAudio
        .split(",")
        .map((language) => language.trim())
        .filter(Boolean),
    [draftRequiredAudio],
  );
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
      indexerRouting: savedSettings?.indexerRoutingOverride ?? null,
      downloadClientRouting: savedSettings?.downloadClientRoutingOverride ?? null,
    }),
    [
      draftQualityProfileId,
      draftRequiredAudioLanguages,
      draftScoringPersona,
      savedSettings,
    ],
  );
  const hasSettingsChanges =
    mode === "new" ||
    (savedSettings !== null &&
      (draftRequiredAudioLanguages.join("\n") !==
        (savedSettings.requiredAudioLanguagesOverride ?? []).join("\n") ||
        settingsDraft.qualityProfileId !== savedSettings.qualityProfileIdOverride ||
        settingsDraft.scoringPersona !== savedSettings.scoringPersonaOverride));
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
    setDraftRequiredAudio("");
    setDraftQualityProfileId(INHERIT_VALUE);
    setDraftScoringPersona(INHERIT_VALUE);
  };

  const handleSaveLibrary = async () => {
    const name = draftName.trim();
    if (!name) {
      return;
    }
    const roots = normalizeRoots(draftRoots);
    setDraftRoots(roots);
    if (mode === "new") {
      const created = await onCreateLibrary({ name, roots, settings: settingsDraft });
      if (created?.id) {
        setMode("existing");
        setActiveLibraryId(created.id);
      }
      return;
    }
    if (activeLibrary) {
      await onUpdateLibrary(activeLibrary.id, { name, roots, settings: settingsDraft });
    }
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
        <CardHeader>
          <CardTitle>{settingsTitle}</CardTitle>
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
                <div className="space-y-2">
                  <Label>{t("settings.librarySlugLabel")}</Label>
                  <div className="truncate rounded-md border border-border bg-muted/50 px-3 py-2 font-mono text-sm text-muted-foreground">
                    {mode === "new" ? t("settings.librarySlugPending") : activeLibrary?.slug}
                  </div>
                </div>
              </div>

              <div className="space-y-3">
                <Label className="block">{t("settings.rootFoldersLabel")}</Label>
                {normalizedDraftRoots.length === 0 && !loading ? (
                  <p className="text-xs text-muted-foreground">{t("settings.rootFoldersEmpty")}</p>
                ) : null}
                <ul className="space-y-2">
                  {sortedFolders.map(({ rf, originalIndex: index }) => (
                    <li key={`${rf.path}-${index}`} className="flex items-center gap-2">
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
                    </li>
                  ))}
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
                  <Label htmlFor="library-audio-languages">
                    {t("settings.libraryRequiredAudioLabel")}
                  </Label>
                  <Input
                    id="library-audio-languages"
                    value={draftRequiredAudio}
                    onChange={(event) => setDraftRequiredAudio(event.target.value)}
                    placeholder={t("settings.libraryRequiredAudioPlaceholder")}
                    disabled={settingsBusy}
                  />
                  <p className="text-xs text-muted-foreground">
                    {savedSettings
                      ? t("settings.libraryEffectiveAudio", {
                          value: savedSettings.requiredAudioLanguages.join(", ") || t("label.none"),
                        })
                      : t("label.loading")}
                  </p>
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
                  <p className="text-xs text-muted-foreground">
                    {savedSettings
                      ? t("settings.libraryEffectiveProfile", {
                          value:
                            qualityProfiles.find(
                              (profile) => profile.id === savedSettings.qualityProfileId,
                            )?.name ?? savedSettings.qualityProfileId,
                        })
                      : t("label.loading")}
                  </p>
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
                  <p className="text-xs text-muted-foreground">
                    {savedSettings
                      ? t("settings.libraryEffectivePersona", {
                          value: t(
                            SCORING_PERSONA_CHOICES.find(
                              (choice) => choice.value === savedSettings.scoringPersona,
                            )?.labelKey ?? "qualityProfile.personaBalanced",
                          ),
                        })
                      : t("label.loading")}
                  </p>
                </div>
              </div>
              {settingsError ? (
                <p className="text-xs text-destructive">{settingsError}</p>
              ) : null}

              <div className="flex flex-wrap items-center gap-2">
                <Button
                  type="button"
                  variant="primary"
                  onClick={handleSaveLibrary}
                  disabled={settingsBusy || !draftName.trim() || !hasDraftChanges}
                >
                  <Save className="mr-1.5 h-4 w-4" />
                  {mode === "new"
                    ? t("settings.libraryCreateButton")
                    : t("settings.librarySaveButton")}
                </Button>
                <Button
                  type="button"
                  variant="outline"
                  onClick={handleDeleteLibrary}
                  disabled={actionBusy || mode === "new" || !activeLibrary || activeLibrary.isDefault}
                >
                  <Trash2 className="mr-1.5 h-4 w-4" />
                  {t("settings.libraryDeleteButton")}
                </Button>
              </div>
            </div>
          ) : null}
        </CardContent>
      </Card>
      <Card>
        <CardHeader>
          <CardTitle>{t("settings.libraryScanTitle")}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3">
          <p className="text-sm text-muted-foreground">{t("settings.libraryScanHelp")}</p>
          <div className="flex flex-wrap items-center gap-3">
            <Button
              type="button"
              onClick={handleScan}
              disabled={scanLoading || actionBusy || mode === "new" || !activeLibrary}
            >
              {scanLoading
                ? t("settings.libraryScanRunning")
                : t("settings.libraryScanButton")}
            </Button>
            {activeLibrary && mode !== "new" ? (
              <span className="text-xs text-muted-foreground">{activeLibrary.name}</span>
            ) : null}
            {scanSummary ? (
              <span className="text-xs text-muted-foreground">
                {libraryScanSummaryText}
              </span>
            ) : null}
          </div>
          {scanNotice ? (
            <p className="text-xs text-destructive">{scanNotice}</p>
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
