import * as React from "react";
import { Edit, Lock, MonitorCog, Plus, Power, PowerOff, RefreshCw, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input, signedIntegerInputProps } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { RenderBooleanIcon } from "@/components/common/boolean-icon";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useTranslate } from "@/lib/context/translate-context";
import { visibleIndexerConfigFields } from "@/lib/types";
import type {
  IndexerRecord,
  IndexerDraft,
  ProviderTypeInfo,
  ConfigFieldDef,
} from "@/lib/types";
import { selectorId } from "@/lib/utils/dom-ids";
import { cn } from "@/lib/utils";
import {
  boxedActionButtonBaseClass,
  boxedActionButtonToneClass,
  type BoxedActionButtonTone,
} from "@/lib/utils/action-button-styles";

type SettingsIndexersSectionProps = {
  editingIndexerId: string | null;
  indexerDraft: IndexerDraft;
  setIndexerDraft: React.Dispatch<React.SetStateAction<IndexerDraft>>;
  submitIndexer: (
    event: React.FormEvent<HTMLFormElement>,
  ) => Promise<void> | void;
  mutatingIndexerId: string | null;
  resetIndexerDraft: () => void;
  settingsIndexerFilter: string;
  setSettingsIndexerFilter: (value: string) => void;
  settingsIndexers: IndexerRecord[];
  editIndexer: (indexer: IndexerRecord) => void;
  toggleIndexerEnabled: (indexer: IndexerRecord) => Promise<void> | void;
  deleteIndexer: (indexer: IndexerRecord) => Promise<void> | void;
  syncIndexer: (indexer: IndexerRecord) => Promise<void> | void;
  providerTypes: ProviderTypeInfo[];
  testIndexerConnection: () => Promise<void> | void;
  isTestingConnection: boolean;
  isEditorOpen: boolean;
  editorMode: "create" | "edit";
  startCreateIndexer: () => void;
};

const FALLBACK_PROVIDER_OPTIONS = [
  { value: "nzbgeek", label: "NZBGeek Indexer" },
  { value: "newznab", label: "Newznab Indexer" },
];

const INDEXER_PROVIDER_LOGOS: Record<string, string> = {
  nzbgeek: "/media-sites/nzbgeek.svg",
  prowlarr: "/media-sites/prowlarr.svg",
};

function getProviderLogoSrc(value: string) {
  return INDEXER_PROVIDER_LOGOS[value.trim().toLowerCase()];
}

function formatIndexerProviderTypeLabel(
  providerType: string,
  t: ReturnType<typeof useTranslate>,
) {
  switch (providerType.trim().toLowerCase()) {
    case "usenet_indexer":
      return `Usenet ${t("settings.pluginCategoryIndexer")}`;
    case "torrent_indexer":
      return `Torrent ${t("settings.pluginCategoryIndexer")}`;
    default:
      return providerType;
  }
}

function IndexerProviderTypeCell({ providerType }: { providerType: string }) {
  const t = useTranslate();
  const logoSrc = getProviderLogoSrc(providerType);
  return (
    <div className="inline-flex items-center gap-2">
      {logoSrc ? (
        <img
          src={logoSrc}
          alt=""
          aria-hidden="true"
          className="h-4 w-4 object-contain"
        />
      ) : null}
      <span>{formatIndexerProviderTypeLabel(providerType, t)}</span>
    </div>
  );
}

function IndexerActionButton({
  label,
  tone,
  className,
  children,
  ...props
}: React.ComponentProps<typeof Button> & {
  label: string;
  tone: Extract<
    BoxedActionButtonTone,
    "edit" | "enabled" | "disabled" | "delete" | "search"
  >;
}) {
  return (
    <Button
      type="button"
      size="icon-sm"
      variant="secondary"
      title={label}
      aria-label={label}
      className={cn(
        boxedActionButtonBaseClass,
        boxedActionButtonToneClass[tone],
        className,
      )}
      {...props}
    >
      {children}
    </Button>
  );
}

function formatRelativeTime(isoDate: string): string {
  const date = new Date(isoDate);
  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  const absDiffMs = Math.abs(diffMs);
  const isFuture = diffMs < 0;

  const minutes = Math.floor(absDiffMs / 60_000);
  const hours = Math.floor(absDiffMs / 3_600_000);
  const days = Math.floor(absDiffMs / 86_400_000);

  let relative: string;
  if (minutes < 1) relative = "just now";
  else if (minutes < 60) relative = `${minutes}m ago`;
  else if (hours < 24) relative = `${hours}h ago`;
  else relative = `${days}d ago`;

  if (isFuture) {
    if (minutes < 60) relative = `in ${minutes}m`;
    else if (hours < 24) relative = `in ${hours}h`;
    else relative = `in ${days}d`;
  }

  return relative;
}

function IndexerStatusCell({ indexer }: { indexer: IndexerRecord }) {
  const t = useTranslate();
  if (!indexer.isEnabled) {
    return <span className="text-muted-foreground">{t("label.disabled")}</span>;
  }

  if (indexer.disabledUntil) {
    const until = new Date(indexer.disabledUntil);
    if (until > new Date()) {
      return (
        <span
          className="text-yellow-600 dark:text-yellow-400"
          title={indexer.disabledUntil}
        >
          {t("settings.indexerDisabledUntil", {
            time: formatRelativeTime(indexer.disabledUntil),
          })}
        </span>
      );
    }
  }

  if (indexer.lastErrorAt) {
    return (
      <span
        className="text-red-600 dark:text-red-400"
        title={indexer.lastErrorAt}
      >
        {t("settings.indexerLastError", {
          time: formatRelativeTime(indexer.lastErrorAt),
        })}
      </span>
    );
  }

  if (indexer.lastQueryAt) {
    return (
      <span className="text-muted-foreground" title={indexer.lastQueryAt}>
        {t("settings.indexerLastSearched", {
          time: formatRelativeTime(indexer.lastQueryAt),
        })}
      </span>
    );
  }

  return (
    <span className="text-muted-foreground">
      {t("settings.indexerNoActivity")}
    </span>
  );
}

function DynamicConfigField({
  field,
  value,
  hasStoredSecretValue = false,
  onChange,
}: {
  field: ConfigFieldDef;
  value: string;
  hasStoredSecretValue?: boolean;
  onChange: (key: string, value: string) => void;
}) {
  const t = useTranslate();
  const fieldId = selectorId("settings-indexer-field", field.key);
  const requiredMarker = field.required ? (
    <span aria-hidden="true" className="text-destructive">
      *
    </span>
  ) : null;

  if (field.fieldType === "bool") {
    return (
      <label className="flex items-center gap-2">
        <Checkbox
          id={fieldId}
          checked={value === "true"}
          onCheckedChange={(checkedValue) =>
            onChange(field.key, checkedValue === true ? "true" : "false")
          }
        />
        <span className="inline-flex items-center gap-2 text-sm">
          {field.label}
          {requiredMarker}
        </span>
        {field.helpText ? (
          <span className="text-xs text-muted-foreground">
            {field.helpText}
          </span>
        ) : null}
      </label>
    );
  }

  if (field.fieldType === "select" && field.options.length > 0) {
    return (
      <label>
        <Label className="mb-2 inline-flex items-center gap-2" htmlFor={fieldId}>
          {field.label}
          {requiredMarker}
        </Label>
        <Select
          value={value || field.defaultValue || ""}
          onValueChange={(v) => onChange(field.key, v)}
        >
          <SelectTrigger id={fieldId} className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {field.options.map((opt) => (
              <SelectItem key={opt.value} value={opt.value}>
                {opt.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {field.helpText ? (
          <p className="mt-1 text-xs text-muted-foreground">{field.helpText}</p>
        ) : null}
      </label>
    );
  }

  if (field.fieldType === "multiline") {
    return (
      <label>
        <Label className="mb-2 inline-flex items-center gap-2" htmlFor={fieldId}>
          {field.label}
          {requiredMarker}
        </Label>
        <Textarea
          id={fieldId}
          value={value}
          onChange={(e) => onChange(field.key, e.target.value)}
          required={field.required && !hasStoredSecretValue}
          placeholder={field.defaultValue ?? ""}
          rows={6}
        />
        {field.helpText ? (
          <p className="mt-1 text-xs text-muted-foreground">{field.helpText}</p>
        ) : null}
      </label>
    );
  }

  return (
    <label>
      <Label className="mb-2 inline-flex items-center gap-2" htmlFor={fieldId}>
        {field.label}
        {requiredMarker}
      </Label>
      <Input
        id={fieldId}
        value={value}
        onChange={(e) => onChange(field.key, e.target.value)}
        {...(field.fieldType === "number" ? signedIntegerInputProps : {})}
        type={
          field.fieldType === "password"
            ? "password"
            : field.fieldType === "number"
              ? "number"
              : "text"
        }
        required={field.required && !hasStoredSecretValue}
        placeholder={
          hasStoredSecretValue
            ? t("form.apiKeyStoredPlaceholder")
            : field.defaultValue ?? ""
        }
      />
      {field.helpText ? (
        <p className="mt-1 text-xs text-muted-foreground">{field.helpText}</p>
      ) : null}
    </label>
  );
}

export function SettingsIndexersSection({
  editingIndexerId,
  indexerDraft,
  setIndexerDraft,
  submitIndexer,
  mutatingIndexerId,
  resetIndexerDraft,
  settingsIndexerFilter,
  setSettingsIndexerFilter,
  settingsIndexers,
  editIndexer,
  toggleIndexerEnabled,
  deleteIndexer,
  syncIndexer,
  providerTypes,
  testIndexerConnection,
  isTestingConnection,
  isEditorOpen,
  editorMode,
  startCreateIndexer,
}: SettingsIndexersSectionProps) {
  const t = useTranslate();
  const normalizedProviderType = indexerDraft.providerType.trim().toLowerCase();
  const isManagedSyncProvider = normalizedProviderType === "prowlarr";
  const isEditing = editorMode === "edit";
  const indexersById = React.useMemo(() => {
    return new Map(settingsIndexers.map((indexer) => [indexer.id, indexer]));
  }, [settingsIndexers]);
  const managedChildCounts = React.useMemo(() => {
    const counts = new Map<string, number>();
    for (const indexer of settingsIndexers) {
      if (indexer.managedParentConfigId) {
        counts.set(
          indexer.managedParentConfigId,
          (counts.get(indexer.managedParentConfigId) ?? 0) + 1,
        );
      }
    }
    return counts;
  }, [settingsIndexers]);

  // Build provider type options from loaded plugins, falling back to hardcoded list
  const providerTypeOptions = React.useMemo(() => {
    const baseOptions =
      providerTypes.length > 0
        ? providerTypes.map((pt) => ({
            value: pt.providerType,
            label: formatIndexerProviderTypeLabel(pt.name, t),
          }))
        : FALLBACK_PROVIDER_OPTIONS;

    if (!normalizedProviderType) {
      return baseOptions;
    }
    if (baseOptions.some((option) => option.value === normalizedProviderType)) {
      return baseOptions;
    }
    return [
      {
        value: normalizedProviderType,
        label: formatIndexerProviderTypeLabel(indexerDraft.providerType, t),
      },
      ...baseOptions,
    ];
  }, [indexerDraft.providerType, normalizedProviderType, providerTypes, t]);

  // Get config fields for the selected provider type
  const selectedProvider = React.useMemo(() => {
    return (
      providerTypes.find((pt) => pt.providerType === normalizedProviderType) ??
      null
    );
  }, [normalizedProviderType, providerTypes]);

  const selectedProviderFields = React.useMemo(
    () =>
      visibleIndexerConfigFields(
        normalizedProviderType,
        (selectedProvider?.configFields ?? []).filter(
          (field) => field.valueSource !== "host_binding",
        ),
      ),
    [normalizedProviderType, selectedProvider],
  );

  const handleConfigValueChange = React.useCallback(
    (key: string, value: string) => {
      setIndexerDraft((prev) => ({
        ...prev,
        configValues: { ...prev.configValues, [key]: value },
      }));
    },
    [setIndexerDraft],
  );

  const handleProviderTypeChange = React.useCallback(
    (nextProviderType: string) => {
      const nextProvider = providerTypes.find(
        (providerType) => providerType.providerType === nextProviderType,
      );
      setIndexerDraft((prev: IndexerDraft) => {
        const previousProvider = providerTypes.find(
          (providerType) => providerType.providerType === prev.providerType,
        );
        const shouldAutofillName =
          prev.name.trim().length === 0 ||
          prev.name === (previousProvider?.name ?? prev.providerType);
        const nextConfigValues: Record<string, string> = {};
        for (const field of nextProvider?.configFields ?? []) {
          if (field.valueSource === "host_binding") {
            continue;
          }
          nextConfigValues[field.key] =
            field.defaultValue ?? (field.fieldType === "bool" ? "false" : "");
        }
        return {
          ...prev,
          providerType: nextProviderType,
          name: shouldAutofillName ? (nextProvider?.name ?? prev.name) : prev.name,
          storedSecretKeys: [],
          configValues: nextConfigValues,
        };
      });
    },
    [providerTypes, setIndexerDraft],
  );

  return (
    <div id="settings-indexers-section" className="space-y-4 text-sm">
      <CardTitle className="flex items-center gap-2 text-base">
        <MonitorCog className="h-4 w-4" />
        {t("settings.indexerProviderSection")}
      </CardTitle>

      <div id="settings-indexers-table-card" className="rounded border border-border">
        <div className="flex items-center justify-between border-b border-border px-3 py-2">
          <CardTitle className="text-base">
            {t("settings.existingIndexers")}
          </CardTitle>
          <Input
            id="settings-indexers-filter"
            value={settingsIndexerFilter}
            onChange={(event) => setSettingsIndexerFilter(event.target.value)}
            placeholder={t("settings.indexerFilterPlaceholder")}
            className="max-w-64"
          />
        </div>
        <div className="overflow-x-auto">
          <Table id="settings-indexers-table">
            <TableHeader>
              <TableRow>
                <TableHead>{t("label.name")}</TableHead>
                <TableHead>{t("settings.indexerProvider")}</TableHead>
                <TableHead>{t("settings.baseUrl")}</TableHead>
                <TableHead className="text-center">
                  {t("label.enabled")}
                </TableHead>
                <TableHead className="text-center">
                  {t("settings.indexerInteractiveSearch")}
                </TableHead>
                <TableHead className="text-center">
                  {t("settings.indexerAutoSearch")}
                </TableHead>
                <TableHead>{t("settings.indexerStatus")}</TableHead>
                <TableHead className="text-right">
                  {t("label.actions")}
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {settingsIndexers.map((indexer) => {
                const parentName = indexer.managedParentConfigId
                  ? indexersById.get(indexer.managedParentConfigId)?.name
                  : null;
                const managedChildCount = managedChildCounts.get(indexer.id) ?? 0;
                return (
                <TableRow
                  key={indexer.id}
                  id={selectorId("settings-indexer-row", indexer.name)}
                  className={indexer.isManaged ? "bg-muted/25" : undefined}
                >
                  <TableCell>
                    <div className="space-y-1">
                      <div className="font-medium">{indexer.name}</div>
                      {indexer.isManaged ? (
                        <div className="flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
                          <span className="inline-flex items-center gap-1 rounded-full border border-amber-200 bg-amber-50 px-2 py-0.5 font-medium text-amber-700 dark:border-amber-500/35 dark:bg-amber-500/12 dark:text-amber-200">
                            <Lock className="h-3 w-3" />
                            {t("settings.managedIndexerBadge")}
                          </span>
                          <span>
                            {parentName
                              ? t("settings.managedByIndexer", { name: parentName })
                              : t("settings.managedByParent")}
                          </span>
                        </div>
                      ) : managedChildCount > 0 ? (
                        <div className="text-xs text-muted-foreground">
                          {t("settings.managesIndexerCount", {
                            count: managedChildCount,
                          })}
                        </div>
                      ) : null}
                    </div>
                  </TableCell>
                  <TableCell>
                    <IndexerProviderTypeCell
                      providerType={indexer.providerType}
                    />
                  </TableCell>
                  <TableCell className="max-w-[260px] truncate">
                    {indexer.baseUrl}
                  </TableCell>
                  <TableCell className="text-center">
                    <RenderBooleanIcon
                      value={indexer.isEnabled}
                      label={`${t("label.enabled")}: ${indexer.name}`}
                    />
                  </TableCell>
                  <TableCell className="text-center">
                    {indexer.supportsManagedChildrenSync ? (
                      <span
                        className="text-muted-foreground"
                        title={t("settings.indexerManagedParentHint")}
                      >
                        —
                      </span>
                    ) : (
                      <RenderBooleanIcon
                        value={indexer.enableInteractiveSearch}
                        label={`${t("settings.indexerInteractiveSearch")}: ${indexer.name}`}
                      />
                    )}
                  </TableCell>
                  <TableCell className="text-center">
                    {indexer.supportsManagedChildrenSync ? (
                      <span
                        className="text-muted-foreground"
                        title={t("settings.indexerManagedParentHint")}
                      >
                        —
                      </span>
                    ) : (
                      <RenderBooleanIcon
                        value={indexer.enableAutoSearch}
                        label={`${t("settings.indexerAutoSearch")}: ${indexer.name}`}
                      />
                    )}
                  </TableCell>
                  <TableCell>
                    <IndexerStatusCell indexer={indexer} />
                  </TableCell>
                  <TableCell className="text-right">
                    <div className="flex justify-end gap-2">
                      {indexer.isManaged ? (
                        <span className="inline-flex items-center gap-1 rounded-full bg-muted px-2 py-1 text-xs text-muted-foreground">
                          <Lock className="h-3 w-3" />
                          {t("settings.managedIndexerReadOnlyShort")}
                        </span>
                      ) : (
                        <>
                          {indexer.supportsManagedChildrenSync ? (
                            <IndexerActionButton
                              id={selectorId("settings-indexer-sync", indexer.name)}
                              tone="search"
                              onClick={() => void syncIndexer(indexer)}
                              disabled={mutatingIndexerId === indexer.id}
                              label={t("settings.indexerSyncNow")}
                            >
                              <RefreshCw className={cn(
                                "h-4 w-4",
                                mutatingIndexerId === indexer.id && "animate-spin",
                              )} />
                            </IndexerActionButton>
                          ) : null}
                          <IndexerActionButton
                            id={selectorId(
                              "settings-indexer-toggle",
                              indexer.name,
                            )}
                            tone={indexer.isEnabled ? "disabled" : "enabled"}
                            onClick={() => void toggleIndexerEnabled(indexer)}
                            disabled={mutatingIndexerId === indexer.id}
                            label={
                              indexer.isEnabled
                                ? t("label.disable")
                                : t("label.enable")
                            }
                          >
                            {indexer.isEnabled ? (
                              <PowerOff className="h-4 w-4" />
                            ) : (
                              <Power className="h-4 w-4" />
                            )}
                          </IndexerActionButton>
                          <IndexerActionButton
                            id={selectorId("settings-indexer-edit", indexer.name)}
                            tone="edit"
                            onClick={() => editIndexer(indexer)}
                            label={t("label.edit")}
                          >
                            <Edit className="h-4 w-4" />
                          </IndexerActionButton>
                          <IndexerActionButton
                            id={selectorId(
                              "settings-indexer-delete",
                              indexer.name,
                            )}
                            tone="delete"
                            onClick={() => void deleteIndexer(indexer)}
                            disabled={mutatingIndexerId === indexer.id}
                            label={
                              mutatingIndexerId === indexer.id
                                ? t("label.deleting")
                                : t("label.delete")
                            }
                          >
                            <Trash2 className="h-4 w-4" />
                          </IndexerActionButton>
                        </>
                      )}
                    </div>
                  </TableCell>
                </TableRow>
                );
              })}
              {settingsIndexers.length === 0 ? (
                <TableRow id="settings-indexers-empty-row">
                  <TableCell colSpan={8} className="text-muted-foreground">
                    {t("settings.noIndexersFound")}
                  </TableCell>
                </TableRow>
              ) : null}
            </TableBody>
          </Table>
        </div>
      </div>

      {isEditorOpen ? (
        <>
          <Card>
            <CardHeader>
              <CardTitle className="text-base">
                {editingIndexerId
                  ? t("settings.indexerUpdate")
                  : t("settings.indexerCreate")}
              </CardTitle>
            </CardHeader>
            <CardContent>
              <form id="settings-indexer-form" className="space-y-3" onSubmit={submitIndexer}>
            <div className="grid gap-3 md:grid-cols-2">
              <label>
                <Label className="mb-2 block" htmlFor="settings-indexer-provider-type">
                  {t("form.providerTypePlaceholder")}
                </Label>
                <Select
                  value={normalizedProviderType || undefined}
                  onValueChange={handleProviderTypeChange}
                >
                  <SelectTrigger id="settings-indexer-provider-type" className="w-full">
                    <SelectValue
                      placeholder={t("form.providerTypePlaceholder")}
                    />
                  </SelectTrigger>
                  <SelectContent>
                    {providerTypeOptions.map((opt) => (
                      <SelectItem key={opt.value} value={opt.value}>
                        {opt.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </label>
              <label>
                <Label className="mb-2 block" htmlFor="settings-indexer-name">{t("label.name")}</Label>
                <Input
                  id="settings-indexer-name"
                  value={indexerDraft.name}
                  onChange={(event) =>
                    setIndexerDraft((prev: IndexerDraft) => ({
                      ...prev,
                      name: event.target.value,
                    }))
                  }
                  required
                  placeholder={t("form.indexerNamePlaceholder")}
                />
              </label>
            </div>

            {selectedProviderFields.length > 0 ? (
              <div className="space-y-3">
                <Label className="text-sm font-medium">
                  {t("settings.indexerConfig")}
                </Label>
                <div className="grid gap-3 md:grid-cols-3">
                  {selectedProviderFields
                    .filter((f) => f.fieldType !== "bool")
                    .map((field) => (
                      <DynamicConfigField
                        key={field.key}
                        field={field}
                        value={
                          indexerDraft.configValues[field.key] ??
                          field.defaultValue ??
                          ""
                        }
                        hasStoredSecretValue={indexerDraft.storedSecretKeys.includes(
                          field.key,
                        )}
                        onChange={handleConfigValueChange}
                      />
                    ))}
                </div>
                {selectedProviderFields.some((f) => f.fieldType === "bool") ? (
                  <div className="flex items-center gap-6">
                    {selectedProviderFields
                      .filter((f) => f.fieldType === "bool")
                      .map((field) => (
                        <DynamicConfigField
                          key={field.key}
                          field={field}
                          value={
                            indexerDraft.configValues[field.key] ??
                            field.defaultValue ??
                            "false"
                          }
                          hasStoredSecretValue={indexerDraft.storedSecretKeys.includes(
                            field.key,
                          )}
                          onChange={handleConfigValueChange}
                        />
                      ))}
                  </div>
                ) : null}
              </div>
            ) : null}

            {isManagedSyncProvider ? (
              <p className="text-sm text-muted-foreground">
                {t("settings.indexerManagedParentHint")}
              </p>
            ) : (
              <div className="flex items-center gap-6">
                <label className="flex items-center gap-2">
                  <Checkbox
                    id="settings-indexer-enable-interactive-search"
                    checked={indexerDraft.enableInteractiveSearch}
                    onCheckedChange={(value) =>
                      setIndexerDraft((prev: IndexerDraft) => ({
                        ...prev,
                        enableInteractiveSearch: value === true,
                      }))
                    }
                  />
                  <span className="text-sm">
                    {t("settings.indexerInteractiveSearch")}
                  </span>
                </label>
                <label className="flex items-center gap-2">
                  <Checkbox
                    id="settings-indexer-enable-auto-search"
                    checked={indexerDraft.enableAutoSearch}
                    onCheckedChange={(value) =>
                      setIndexerDraft((prev: IndexerDraft) => ({
                        ...prev,
                        enableAutoSearch: value === true,
                      }))
                    }
                  />
                  <span className="text-sm">
                    {t("settings.indexerAutoSearch")}
                  </span>
                </label>
              </div>
            )}
            <div className="flex gap-2">
              <Button id="settings-indexer-save" type="submit" disabled={mutatingIndexerId === "new"}>
                {mutatingIndexerId === "new"
                  ? t("label.saving")
                  : editingIndexerId
                    ? t("settings.indexerUpdate")
                    : t("settings.indexerCreate")}
              </Button>
              <Button
                id="settings-indexer-test-connection"
                type="button"
                variant="outline"
                onClick={() => void testIndexerConnection()}
                disabled={isTestingConnection}
              >
                {isTestingConnection
                  ? t("status.testingIndexerConnection")
                  : t("label.testConnection")}
              </Button>
              <Button
                id="settings-indexer-cancel"
                type="button"
                variant="outline"
                onClick={resetIndexerDraft}
              >
                {t("label.cancel")}
              </Button>
            </div>
              </form>
            </CardContent>
          </Card>
          {isEditing ? (
            <div className="flex justify-center">
              <Button
                id="settings-indexer-create"
                type="button"
                size="lg"
                onClick={startCreateIndexer}
                disabled={mutatingIndexerId !== null}
                className="h-12 border border-emerald-500/30 bg-emerald-500/15 px-5 text-base font-semibold text-emerald-100 hover:bg-emerald-500/25 hover:text-emerald-50"
              >
                <Plus className="h-5 w-5" />
                {t("settings.indexerCreateNew")}
              </Button>
            </div>
          ) : null}
        </>
      ) : (
        <div className="flex justify-center">
          <Button
            id="settings-indexer-create"
            type="button"
            size="lg"
            onClick={startCreateIndexer}
            className="h-12 border border-emerald-500/30 bg-emerald-500/15 px-5 text-base font-semibold text-emerald-100 hover:bg-emerald-500/25 hover:text-emerald-50"
          >
            <Plus className="h-5 w-5" />
            {t("settings.indexerCreateNew")}
          </Button>
        </div>
      )}
    </div>
  );
}
