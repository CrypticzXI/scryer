import { useMemo, useState } from "react";
import { ArrowLeft, Ban, Check, FolderOpen, Loader2, Trash2 } from "lucide-react";

import { FolderBrowserDialog } from "@/components/setup/folder-browser-dialog";
import { Button } from "@/components/ui/button";
import type { ConfigFieldDef } from "@/lib/types";
import type { ExternalImportPreview } from "@/lib/types/external-import";

type ImportFacet = "movie" | "series" | "anime";
type BrowseTarget = ImportFacet | null;

function providerRequiresApiKey(fields: ConfigFieldDef[]): boolean {
  return fields.some((field) => {
    const normalizedKey = field.key.trim().toLowerCase();
    const normalizedFieldType = field.fieldType.trim().toLowerCase();
    return field.required && (
      normalizedKey === "api_key" ||
      normalizedKey === "apikey" ||
      (
        (normalizedFieldType === "password" || normalizedFieldType === "secret")
        && normalizedKey.includes("api")
      )
    );
  });
}

interface SetupImportReviewViewProps {
  t: (key: string) => string;
  preview: ExternalImportPreview;
  selectedMoviesPaths: string[];
  selectedSeriesPaths: string[];
  selectedAnimePaths: string[];
  customMoviesPaths: string[];
  customSeriesPaths: string[];
  customAnimePaths: string[];
  selectedDcKeys: Set<string>;
  selectedIdxKeys: Set<string>;
  apiKeyOverrides: Map<string, string>;
  indexerProviderConfigFieldsByType: Map<string, ConfigFieldDef[]>;
  onToggleMoviesPath: (path: string) => void;
  onToggleSeriesPath: (path: string) => void;
  onToggleAnimePath: (path: string) => void;
  onAddCustomMoviesPath: (path: string) => void;
  onAddCustomSeriesPath: (path: string) => void;
  onAddCustomAnimePath: (path: string) => void;
  onRemoveCustomMoviesPath: (path: string) => void;
  onRemoveCustomSeriesPath: (path: string) => void;
  onRemoveCustomAnimePath: (path: string) => void;
  onToggleDc: (dedupKey: string) => void;
  onToggleIdx: (dedupKey: string) => void;
  onSetApiKey: (dedupKey: string, apiKey: string) => void;
  onImport: () => void;
  onBack: () => void;
  importing: boolean;
  error: string | null;
}

export function SetupImportReviewView({
  t,
  preview,
  selectedMoviesPaths,
  selectedSeriesPaths,
  selectedAnimePaths,
  customMoviesPaths,
  customSeriesPaths,
  customAnimePaths,
  selectedDcKeys,
  selectedIdxKeys,
  apiKeyOverrides,
  indexerProviderConfigFieldsByType,
  onToggleMoviesPath,
  onToggleSeriesPath,
  onToggleAnimePath,
  onAddCustomMoviesPath,
  onAddCustomSeriesPath,
  onAddCustomAnimePath,
  onRemoveCustomMoviesPath,
  onRemoveCustomSeriesPath,
  onRemoveCustomAnimePath,
  onToggleDc,
  onToggleIdx,
  onSetApiKey,
  onImport,
  onBack,
  importing,
  error,
}: SetupImportReviewViewProps) {
  const [browseTarget, setBrowseTarget] = useState<BrowseTarget>(null);

  const radarrFolders = useMemo(
    () => preview.rootFolders.filter((f) => f.source === "radarr"),
    [preview.rootFolders],
  );
  const sonarrFolders = useMemo(
    () => preview.rootFolders.filter((f) => f.source === "sonarr"),
    [preview.rootFolders],
  );
  const hasAnySelection =
    selectedMoviesPaths.length > 0 ||
    selectedSeriesPaths.length > 0 ||
    selectedAnimePaths.length > 0 ||
    customMoviesPaths.length > 0 ||
    customSeriesPaths.length > 0 ||
    customAnimePaths.length > 0 ||
    selectedDcKeys.size > 0 ||
    selectedIdxKeys.size > 0;

  const browserInitialPath =
    browseTarget === "movie"
      ? customMoviesPaths.at(-1) ?? selectedMoviesPaths[0] ?? radarrFolders[0]?.path ?? "/"
      : browseTarget === "series"
        ? customSeriesPaths.at(-1) ?? selectedSeriesPaths[0] ?? sonarrFolders[0]?.path ?? "/"
        : browseTarget === "anime"
          ? customAnimePaths.at(-1) ?? selectedAnimePaths[0] ?? sonarrFolders[0]?.path ?? "/"
          : "/";

  const browserTitle =
    browseTarget === "movie"
      ? t("setup.facetMovies")
      : browseTarget === "series"
        ? t("setup.facetSeries")
        : browseTarget === "anime"
          ? t("setup.facetAnime")
          : t("setup.browse");

  const handleBrowseSelect = (path: string) => {
    if (browseTarget === "movie") {
      onAddCustomMoviesPath(path);
      return;
    }
    if (browseTarget === "series") {
      onAddCustomSeriesPath(path);
      return;
    }
    if (browseTarget === "anime") {
      onAddCustomAnimePath(path);
    }
  };

  return (
    <div className="w-full space-y-6">
      <div className="text-center">
        <h2 className="mb-2 text-xl font-semibold">{t("setup.reviewTitle")}</h2>
        <p className="text-sm text-muted-foreground">{t("setup.reviewDescription")}</p>
      </div>

      <div className="flex items-center justify-center gap-3">
        {preview.sonarrConnected ? (
          <span className="inline-flex items-center gap-1.5 rounded-full bg-blue-500/10 px-3 py-1 text-xs font-medium text-blue-600 dark:text-blue-400">
            <Check className="h-3 w-3" />
            Sonarr {preview.sonarrVersion ? `v${preview.sonarrVersion}` : ""} {t("setup.connected")}
          </span>
        ) : null}
        {preview.radarrConnected ? (
          <span className="inline-flex items-center gap-1.5 rounded-full bg-amber-500/10 px-3 py-1 text-xs font-medium text-amber-600 dark:text-amber-400">
            <Check className="h-3 w-3" />
            Radarr {preview.radarrVersion ? `v${preview.radarrVersion}` : ""} {t("setup.connected")}
          </span>
        ) : null}
      </div>

      {(preview.radarrConnected || preview.sonarrConnected) ? (
        <Section title={t("setup.mediaPathsSection")}>
          {preview.radarrConnected ? (
            <ImportPathFacetSection
              label={t("setup.facetMovies")}
              importedFolders={radarrFolders}
              selectedImportedPaths={selectedMoviesPaths}
              customPaths={customMoviesPaths}
              onToggleImported={onToggleMoviesPath}
              onRemoveCustom={onRemoveCustomMoviesPath}
              onAddCustom={() => setBrowseTarget("movie")}
              t={t}
            />
          ) : null}
          {preview.sonarrConnected ? (
            <ImportPathFacetSection
              label={t("setup.facetSeries")}
              importedFolders={sonarrFolders}
              selectedImportedPaths={selectedSeriesPaths}
              customPaths={customSeriesPaths}
              onToggleImported={onToggleSeriesPath}
              onRemoveCustom={onRemoveCustomSeriesPath}
              onAddCustom={() => setBrowseTarget("series")}
              t={t}
            />
          ) : null}
          {preview.sonarrConnected ? (
            <ImportPathFacetSection
              label={t("setup.facetAnime")}
              importedFolders={sonarrFolders}
              selectedImportedPaths={selectedAnimePaths}
              customPaths={customAnimePaths}
              onToggleImported={onToggleAnimePath}
              onRemoveCustom={onRemoveCustomAnimePath}
              onAddCustom={() => setBrowseTarget("anime")}
              t={t}
            />
          ) : null}
        </Section>
      ) : null}

      {preview.downloadClients.length > 0 ? (
        <Section title={t("setup.downloadClientsSection")}>
          {preview.downloadClients.map((dc) => {
            const needsApiKey =
              dc.supported &&
              dc.apiKey === null &&
              (dc.scryerClientType === "sabnzbd" || dc.scryerClientType === "weaver");
            const isSelected = selectedDcKeys.has(dc.dedupKey);
            const sabUrl = dc.host
              ? `${dc.useSsl ? "https" : "http"}://${dc.host}${dc.port ? `:${dc.port}` : ""}${dc.urlBase ? `/${dc.urlBase.replace(/^\//, "")}` : ""}/config/general/`
              : null;
            return (
              <div key={dc.dedupKey}>
                <label
                  className={`flex items-center gap-3 rounded px-2 py-2 text-sm ${
                    dc.supported ? "cursor-pointer hover:bg-muted" : "cursor-not-allowed opacity-50"
                  }`}
                >
                  <input
                    type="checkbox"
                    checked={isSelected}
                    onChange={() => onToggleDc(dc.dedupKey)}
                    disabled={!dc.supported}
                    className="accent-primary"
                  />
                  <div className="flex-1">
                    <span className="font-medium">{dc.name}</span>
                    <span className="ml-2 text-xs text-muted-foreground">
                      {dc.implementation}
                      {dc.host ? ` @ ${dc.host}${dc.port ? `:${dc.port}` : ""}` : ""}
                    </span>
                  </div>
                  <SourceBadges sources={dc.sources} t={t} />
                  {!dc.supported ? (
                    <span className="rounded bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
                      {t("setup.notSupported")}
                    </span>
                  ) : null}
                </label>
                {needsApiKey && isSelected ? (
                  <div className="mb-1 ml-8 space-y-1">
                    <p className="text-xs text-muted-foreground">{t("setup.apiKeyMasked")}</p>
                    <input
                      type="password"
                      value={apiKeyOverrides.get(dc.dedupKey) ?? ""}
                      onChange={(e) => onSetApiKey(dc.dedupKey, e.target.value)}
                      placeholder={t("setup.apiKeyPlaceholder")}
                      className="w-full rounded border border-border bg-background px-2 py-1 font-mono text-xs outline-none focus:ring-1 focus:ring-primary"
                    />
                    {sabUrl ? (
                      <a
                        href={sabUrl}
                        target="_blank"
                        rel="noopener noreferrer"
                        className="inline-block text-xs text-primary underline underline-offset-2"
                      >
                        {t("setup.apiKeyHelpLink")}
                      </a>
                    ) : null}
                  </div>
                ) : null}
              </div>
            );
          })}
        </Section>
      ) : null}

      {preview.indexers.length > 0 ? (
        <Section title={t("setup.indexersSection")}>
          {preview.indexers.map((idx) => {
            const providerConfigFields =
              idx.scryerProviderType === null
                ? []
                : (indexerProviderConfigFieldsByType.get(idx.scryerProviderType) ?? []);
            const needsApiKey =
              idx.supported &&
              idx.apiKey === null &&
              providerRequiresApiKey(providerConfigFields);
            const isSelected = selectedIdxKeys.has(idx.dedupKey);

            return (
              <div key={idx.dedupKey}>
                <label
                  className={`flex items-center gap-3 rounded px-2 py-2 text-sm ${
                    idx.supported ? "cursor-pointer hover:bg-muted" : "cursor-not-allowed opacity-50"
                  }`}
                >
                  <input
                    type="checkbox"
                    checked={isSelected}
                    onChange={() => onToggleIdx(idx.dedupKey)}
                    disabled={!idx.supported}
                    className="accent-primary"
                  />
                  <div className="flex-1">
                    <span className="font-medium">{idx.name}</span>
                    <span className="ml-2 text-xs text-muted-foreground">
                      {idx.implementation}
                      {idx.baseUrl ? ` @ ${idx.baseUrl}` : ""}
                    </span>
                  </div>
                  <SourceBadges sources={idx.sources} t={t} />
                  {!idx.supported ? (
                    <span className="rounded bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
                      {t("setup.notSupported")}
                    </span>
                  ) : null}
                </label>
                {needsApiKey && isSelected ? (
                  <div className="mb-1 ml-8 space-y-1">
                    <p className="text-xs text-muted-foreground">{t("setup.apiKeyMasked")}</p>
                    <input
                      type="password"
                      value={apiKeyOverrides.get(idx.dedupKey) ?? ""}
                      onChange={(e) => onSetApiKey(idx.dedupKey, e.target.value)}
                      placeholder={t("setup.apiKeyPlaceholder")}
                      className="w-full rounded border border-border bg-background px-2 py-1 font-mono text-xs outline-none focus:ring-1 focus:ring-primary"
                    />
                  </div>
                ) : null}
              </div>
            );
          })}
        </Section>
      ) : null}

      {preview.downloadClients.length === 0 &&
      preview.indexers.length === 0 &&
      preview.rootFolders.length === 0 ? (
        <p className="py-4 text-center text-sm text-muted-foreground">
          <Ban className="mb-1 inline-block h-4 w-4" /> {t("setup.noItemsFound")}
        </p>
      ) : null}

      <p className="text-center text-xs text-muted-foreground">
        {t("setup.customFormatsHint")}
      </p>

      {error ? (
        <p className="text-center text-sm text-destructive">{error}</p>
      ) : null}

      <div className="flex items-center justify-between">
        <Button variant="ghost" onClick={onBack} disabled={importing}>
          <ArrowLeft className="mr-2 h-4 w-4" />
          {t("setup.back")}
        </Button>
        <Button onClick={onImport} disabled={!hasAnySelection || importing}>
          {importing ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
          {importing ? t("setup.importing") : t("setup.importSelected")}
        </Button>
      </div>

      <FolderBrowserDialog
        open={browseTarget !== null}
        onOpenChange={(open) => {
          if (!open) {
            setBrowseTarget(null);
          }
        }}
        onSelect={handleBrowseSelect}
        initialPath={browserInitialPath}
        title={`${t("settings.rootFolderAdd")} · ${browserTitle}`}
      />
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="space-y-2 rounded-lg border border-border p-4">
      <h3 className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
        {title}
      </h3>
      {children}
    </div>
  );
}

function ImportPathFacetSection({
  label,
  importedFolders,
  selectedImportedPaths,
  customPaths,
  onToggleImported,
  onRemoveCustom,
  onAddCustom,
  t,
}: {
  label: string;
  importedFolders: ExternalImportPreview["rootFolders"];
  selectedImportedPaths: string[];
  customPaths: string[];
  onToggleImported: (path: string) => void;
  onRemoveCustom: (path: string) => void;
  onAddCustom: () => void;
  t: (key: string) => string;
}) {
  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between gap-3">
        <p className="text-lg font-semibold text-foreground">{label}</p>
        <Button type="button" variant="outline" size="sm" onClick={onAddCustom}>
          <FolderOpen className="mr-1.5 h-4 w-4" />
          {t("settings.rootFolderAdd")}
        </Button>
      </div>

      <div className="space-y-1">
        {importedFolders.map((folder) => (
          <label
            key={folder.path}
            className="flex cursor-pointer items-center gap-2 rounded px-2 py-1.5 text-sm hover:bg-muted"
          >
            <input
              type="checkbox"
              checked={selectedImportedPaths.includes(folder.path)}
              onChange={() => onToggleImported(folder.path)}
              className="accent-primary"
            />
            <code className="min-w-0 flex-1 truncate text-xs">{folder.path}</code>
            <SourceBadges sources={[folder.source]} t={t} />
          </label>
        ))}

        {customPaths.map((path) => (
          <div
            key={path}
            className="flex items-center gap-2 rounded px-2 py-1.5 text-sm hover:bg-muted"
          >
            <Check className="h-4 w-4 shrink-0 text-primary" />
            <code className="min-w-0 flex-1 truncate text-xs">{path}</code>
            <span className="rounded bg-primary/10 px-1.5 py-0.5 text-[10px] font-medium text-primary">
              Scryer
            </span>
            <Button
              type="button"
              variant="ghost"
              size="icon"
              className="h-7 w-7 shrink-0 text-destructive hover:text-destructive"
              onClick={() => onRemoveCustom(path)}
              aria-label={t("label.remove")}
            >
              <Trash2 className="h-4 w-4" />
            </Button>
          </div>
        ))}
      </div>
    </div>
  );
}

function SourceBadges({ sources, t }: { sources: string[]; t: (key: string) => string }) {
  return (
    <span className="flex gap-1">
      {sources.map((source) => {
        const isSonarr = source === "sonarr";
        return (
          <span
            key={source}
            className={`rounded px-1.5 py-0.5 text-[10px] font-medium ${
              isSonarr
                ? "bg-blue-500/10 text-blue-600 dark:text-blue-400"
                : "bg-amber-500/10 text-amber-600 dark:text-amber-400"
            }`}
          >
            {isSonarr ? t("setup.fromSonarr") : t("setup.fromRadarr")}
          </span>
        );
      })}
    </span>
  );
}
