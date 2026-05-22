import * as React from "react";
import {
  CircleAlert,
  Edit,
  Loader2,
  Plus,
  PlugZap,
  Power,
  PowerOff,
  Trash2,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Textarea } from "@/components/ui/textarea";
import { useTranslate } from "@/lib/context/translate-context";
import type {
  ConfigFieldDef,
  SubtitleProviderConfigRecord,
  SubtitleProviderDraft,
  SubtitleProviderTypeInfo,
} from "@/lib/types";
import { cn } from "@/lib/utils";
import {
  boxedActionButtonBaseClass,
  boxedActionButtonToneClass,
  type BoxedActionButtonTone,
} from "@/lib/utils/action-button-styles";
import { selectorId } from "@/lib/utils/dom-ids";

type Props = {
  editingProviderId: string | null;
  providerDraft: SubtitleProviderDraft;
  setProviderDraft: React.Dispatch<React.SetStateAction<SubtitleProviderDraft>>;
  submitProvider: (
    event: React.FormEvent<HTMLFormElement>,
  ) => Promise<void> | void;
  mutatingProviderId: string | null;
  resetProviderDraft: () => void;
  providerConfigs: SubtitleProviderConfigRecord[];
  editProvider: (provider: SubtitleProviderConfigRecord) => void;
  toggleProviderEnabled: (
    provider: SubtitleProviderConfigRecord,
  ) => Promise<void> | void;
  deleteProvider: (provider: SubtitleProviderConfigRecord) => Promise<void> | void;
  providerTypes: SubtitleProviderTypeInfo[];
  testProviderConnection: () => Promise<void> | void;
  isTestingConnection: boolean;
  isEditorOpen: boolean;
  editorMode: "create" | "edit";
  startCreateProvider: () => void;
};

const SUBTITLE_FACETS = [
  { value: "movie", labelKey: "label.movies" },
  { value: "series", labelKey: "label.series" },
  { value: "anime", labelKey: "label.anime" },
] as const;

function looksLikeSecretConfigKey(key: string): boolean {
  const normalized = key.trim().toLowerCase();
  return (
    normalized === "api_key" ||
    normalized === "apikey" ||
    normalized.includes("api_key") ||
    normalized.includes("password") ||
    normalized.includes("secret") ||
    normalized.includes("token")
  );
}

function SubtitleProviderActionButton({
  label,
  tone,
  className,
  children,
  ...props
}: React.ComponentProps<typeof Button> & {
  label: string;
  tone: Extract<BoxedActionButtonTone, "edit" | "enabled" | "disabled" | "delete">;
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

function SubtitleProviderStatusCell({
  provider,
}: {
  provider: SubtitleProviderConfigRecord;
}) {
  const t = useTranslate();
  if (!provider.isEnabled) {
    return <span className="text-muted-foreground">{t("label.disabled")}</span>;
  }

  if (provider.disabledUntil) {
    const until = new Date(provider.disabledUntil);
    if (until > new Date()) {
      return (
        <span
          className="text-yellow-600 dark:text-yellow-400"
          title={provider.disabledUntil}
        >
          {t("settings.subtitleProviderDisabledUntil", {
            time: formatRelativeTime(provider.disabledUntil),
          })}
        </span>
      );
    }
  }

  if (provider.lastErrorAt) {
    return (
      <div className="space-y-1">
        <span
          className="text-red-600 dark:text-red-400"
          title={provider.lastErrorAt}
        >
          {t("settings.subtitleProviderLastError", {
            time: formatRelativeTime(provider.lastErrorAt),
          })}
        </span>
        {provider.lastError ? (
          <p className="max-w-sm text-xs text-muted-foreground">
            {provider.lastError}
          </p>
        ) : null}
      </div>
    );
  }

  if (provider.lastHealthStatus) {
    return (
      <span className="text-muted-foreground">
        {provider.lastHealthStatus}
      </span>
    );
  }

  return (
    <span className="text-muted-foreground">
      {t("settings.subtitleProviderNoActivity")}
    </span>
  );
}

function SubtitleProviderFacetChips({
  facets,
}: {
  facets: SubtitleProviderConfigRecord["enabledFacets"];
}) {
  const t = useTranslate();
  if (facets.length === 0) {
    return <span className="text-muted-foreground">-</span>;
  }

  return (
    <div className="flex flex-wrap gap-1">
      {facets.map((facet) => {
        const labelKey =
          SUBTITLE_FACETS.find((item) => item.value === facet)?.labelKey ??
          "label.unknown";
        return (
          <span
            key={facet}
            className="rounded bg-muted px-1.5 py-0.5 text-[11px] text-muted-foreground"
          >
            {t(labelKey)}
          </span>
        );
      })}
    </div>
  );
}


function DynamicSubtitleConfigField({
  field,
  value,
  onChange,
  hasStoredSecretValue,
}: {
  field: ConfigFieldDef;
  value: string;
  onChange: (key: string, value: string) => void;
  hasStoredSecretValue: boolean;
}) {
  const t = useTranslate();
  const fieldId = selectorId("settings-subtitle-provider-config", field.key);
  const requiredMarker = field.required ? (
    <span aria-hidden="true" className="text-destructive">
      *
    </span>
  ) : null;

  if (field.fieldType === "bool") {
    return (
      <label className="flex items-center gap-2 rounded-lg border border-border/60 bg-card/40 px-3 py-2">
        <input
          id={fieldId}
          type="checkbox"
          checked={value === "true"}
          onChange={(event) =>
            onChange(field.key, event.target.checked ? "true" : "false")
          }
          className="accent-primary"
        />
        <div className="space-y-1">
          <span className="inline-flex items-center gap-2 text-sm font-medium">
            {field.label}
            {requiredMarker}
          </span>
          {field.helpText ? (
            <p className="text-xs text-muted-foreground">{field.helpText}</p>
          ) : null}
        </div>
      </label>
    );
  }

  if (field.fieldType === "select" && field.options.length > 0) {
    return (
      <label>
        <Label htmlFor={fieldId} className="mb-2 inline-flex items-center gap-2">
          {field.label}
          {requiredMarker}
        </Label>
        <Select
          value={value || field.defaultValue || ""}
          onValueChange={(nextValue) => onChange(field.key, nextValue)}
        >
          <SelectTrigger id={fieldId} className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {field.options.map((option) => (
              <SelectItem key={option.value} value={option.value}>
                {option.label}
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
        <Label htmlFor={fieldId} className="mb-2 inline-flex items-center gap-2">
          {field.label}
          {requiredMarker}
        </Label>
        <Textarea
          id={fieldId}
          value={value}
          onChange={(event) => onChange(field.key, event.target.value)}
          required={field.required}
          placeholder={field.defaultValue ?? ""}
          rows={6}
        />
        {field.helpText ? (
          <p className="mt-1 text-xs text-muted-foreground">{field.helpText}</p>
        ) : null}
      </label>
    );
  }

  const isSecretField =
    field.fieldType === "password" ||
    field.fieldType === "secret" ||
    looksLikeSecretConfigKey(field.key);

  return (
    <label>
      <Label htmlFor={fieldId} className="mb-2 inline-flex items-center gap-2">
        {field.label}
        {requiredMarker}
      </Label>
      <Input
        id={fieldId}
        value={value}
        onChange={(event) => onChange(field.key, event.target.value)}
        type={isSecretField ? "password" : "text"}
        required={field.required && !hasStoredSecretValue}
        placeholder={
          isSecretField && hasStoredSecretValue
            ? t("settings.subtitleProviderSecretStored")
            : (field.defaultValue ?? "")
        }
      />
      {field.helpText ? (
        <p className="mt-1 text-xs text-muted-foreground">{field.helpText}</p>
      ) : null}
    </label>
  );
}

export function SettingsSubtitleProvidersSection({
  editingProviderId,
  providerDraft,
  setProviderDraft,
  submitProvider,
  mutatingProviderId,
  resetProviderDraft,
  providerConfigs,
  editProvider,
  toggleProviderEnabled,
  deleteProvider,
  providerTypes,
  testProviderConnection,
  isTestingConnection,
  isEditorOpen,
  editorMode,
  startCreateProvider,
}: Props) {
  const t = useTranslate();
  const normalizedProviderType = providerDraft.providerType.trim().toLowerCase();
  const isEditing = editorMode === "edit";

  const providerTypeOptions = React.useMemo(() => {
    const baseOptions = providerTypes.map((providerType) => ({
      value: providerType.providerType,
      label: providerType.name,
    }));

    if (!normalizedProviderType) {
      return baseOptions;
    }

    if (baseOptions.some((option) => option.value === normalizedProviderType)) {
      return baseOptions;
    }

    return [
      { value: normalizedProviderType, label: providerDraft.providerType },
      ...baseOptions,
    ];
  }, [normalizedProviderType, providerDraft.providerType, providerTypes]);

  const selectedProvider = React.useMemo(
    () =>
      providerTypes.find(
        (providerType) => providerType.providerType === normalizedProviderType,
      ) ?? null,
    [normalizedProviderType, providerTypes],
  );

  const selectedProviderFields = React.useMemo(
    () =>
      (selectedProvider?.configFields ?? []).filter(
        (field) => field.valueSource !== "host_binding",
      ),
    [selectedProvider],
  );

  const handleProviderTypeChange = React.useCallback(
    (nextProviderType: string) => {
      const nextProvider = providerTypes.find(
        (providerType) => providerType.providerType === nextProviderType,
      );
      setProviderDraft((previous) => {
        const previousProvider = providerTypes.find(
          (providerType) => providerType.providerType === previous.providerType,
        );
        const shouldAutofillName =
          previous.name.trim().length === 0 ||
          previous.name === (previousProvider?.name ?? previous.providerType);
        const nextConfigValues: Record<string, string> = {};
        for (const field of nextProvider?.configFields ?? []) {
          if (field.valueSource === "host_binding") {
            continue;
          }
          nextConfigValues[field.key] =
            previous.persistedConfigValues[field.key] ??
            field.defaultValue ??
            (field.fieldType === "bool" ? "false" : "");
        }
        return {
          ...previous,
          providerType: nextProviderType,
          name: shouldAutofillName
            ? (nextProvider?.name ?? previous.name)
            : previous.name,
          configValues: nextConfigValues,
          persistedConfigValues: {},
          storedSecretKeys: [],
          configDirty: true,
          enabledFacets: nextProvider?.recommendedFacets ?? [],
        };
      });
    },
    [providerTypes, setProviderDraft],
  );

  const handleConfigValueChange = React.useCallback(
    (key: string, value: string) => {
      setProviderDraft((previous) => ({
        ...previous,
        configValues: {
          ...previous.configValues,
          [key]: value,
        },
        configDirty: true,
      }));
    },
    [setProviderDraft],
  );

  const handleFacetToggle = React.useCallback(
    (facet: "movie" | "series" | "anime", checked: boolean) => {
      setProviderDraft((previous) => {
        const current = new Set(previous.enabledFacets);
        if (checked) {
          current.add(facet);
        } else {
          current.delete(facet);
        }
        return {
          ...previous,
          enabledFacets: SUBTITLE_FACETS.map((item) => item.value).filter((value) =>
            current.has(value),
          ),
        };
      });
    },
    [setProviderDraft],
  );

  return (
    <div id="settings-subtitle-providers-section" className="space-y-4 rounded-xl border border-border/60 bg-card/30 p-4">
      <CardTitle className="flex items-center gap-2 text-base">
        <PlugZap className="h-4 w-4" />
        {t("settings.subtitleProviders")}
      </CardTitle>

      <div className="rounded border border-border">
        <div className="border-b border-border px-3 py-2">
          <CardTitle className="text-base">
            {t("settings.existingSubtitleProviders")}
          </CardTitle>
        </div>
        <div className="overflow-x-auto">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("label.name")}</TableHead>
                <TableHead>{t("settings.subtitleProviderType")}</TableHead>
                <TableHead className="text-center">{t("label.enabled")}</TableHead>
                <TableHead>{t("settings.subtitleProviderFacets")}</TableHead>
                <TableHead>{t("settings.subtitleProviderStatus")}</TableHead>
                <TableHead className="text-right">{t("label.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {providerConfigs.length === 0 ? (
                <TableRow>
                  <TableCell
                    colSpan={6}
                    className="text-center text-sm text-muted-foreground"
                  >
                    {t("settings.subtitleProviderEmpty")}
                  </TableCell>
                </TableRow>
              ) : (
                providerConfigs.map((provider) => (
                  <TableRow
                    key={provider.id}
                    id={selectorId("settings-subtitle-provider-row", provider.id)}
                  >
                    <TableCell className="font-medium">{provider.name}</TableCell>
                    <TableCell>{provider.providerType}</TableCell>
                    <TableCell className="text-center">
                      {provider.isEnabled ? t("label.enabled") : t("label.disabled")}
                    </TableCell>
                    <TableCell>
                      <SubtitleProviderFacetChips facets={provider.enabledFacets ?? []} />
                    </TableCell>
                    <TableCell>
                      <SubtitleProviderStatusCell provider={provider} />
                    </TableCell>
                    <TableCell className="text-right">
                      <div className="inline-flex items-center gap-2">
                        <SubtitleProviderActionButton
                          id={selectorId("settings-subtitle-provider-edit", provider.id)}
                          label={t("label.edit")}
                          tone="edit"
                          onClick={() => editProvider(provider)}
                          disabled={mutatingProviderId !== null}
                        >
                          <Edit className="h-4 w-4" />
                        </SubtitleProviderActionButton>
                        <SubtitleProviderActionButton
                          id={selectorId("settings-subtitle-provider-toggle", provider.id)}
                          label={
                            provider.isEnabled ? t("label.disable") : t("label.enable")
                          }
                          tone={provider.isEnabled ? "enabled" : "disabled"}
                          onClick={() => void toggleProviderEnabled(provider)}
                          disabled={mutatingProviderId !== null}
                        >
                          {provider.isEnabled ? (
                            <Power className="h-4 w-4" />
                          ) : (
                            <PowerOff className="h-4 w-4" />
                          )}
                        </SubtitleProviderActionButton>
                        <SubtitleProviderActionButton
                          id={selectorId("settings-subtitle-provider-delete", provider.id)}
                          label={t("label.delete")}
                          tone="delete"
                          onClick={() => void deleteProvider(provider)}
                          disabled={mutatingProviderId !== null}
                        >
                          <Trash2 className="h-4 w-4" />
                        </SubtitleProviderActionButton>
                      </div>
                    </TableCell>
                  </TableRow>
                ))
              )}
            </TableBody>
          </Table>
        </div>
      </div>

      {isEditorOpen ? (
        <>
          <form
            id="settings-subtitle-provider-form"
            className="space-y-4 rounded-lg border border-border/60 p-4"
            onSubmit={submitProvider}
          >
            <div className="flex items-center justify-between gap-3">
              <CardTitle className="text-base">
                {isEditing
                  ? t("settings.subtitleProviderEdit")
                  : t("settings.subtitleProviderCreate")}
              </CardTitle>
              {mutatingProviderId ? (
                <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
              ) : null}
            </div>

            <div className="grid gap-4 md:grid-cols-2">
              <label>
                <Label className="mb-2 block">{t("settings.subtitleProviderType")}</Label>
                <Select
                  value={normalizedProviderType}
                  onValueChange={handleProviderTypeChange}
                >
                  <SelectTrigger id="settings-subtitle-provider-type" className="w-full">
                    <SelectValue placeholder={t("form.providerTypePlaceholder")} />
                  </SelectTrigger>
                  <SelectContent>
                    {providerTypeOptions.map((option) => (
                      <SelectItem key={option.value} value={option.value}>
                        {option.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </label>

              <label>
                <Label className="mb-2 block">{t("label.name")}</Label>
                <Input
                  id="settings-subtitle-provider-name"
                  value={providerDraft.name}
                  onChange={(event) =>
                    setProviderDraft((previous) => ({
                      ...previous,
                      name: event.target.value,
                    }))
                  }
                  placeholder={t("settings.subtitleProviderNamePlaceholder")}
                  required
                />
              </label>
            </div>

            <label className="flex items-center gap-2">
              <input
                id="settings-subtitle-provider-enabled"
                type="checkbox"
                checked={providerDraft.isEnabled}
                onChange={(event) =>
                  setProviderDraft((previous) => ({
                    ...previous,
                    isEnabled: event.target.checked,
                  }))
                }
                className="accent-primary"
              />
              <span className="text-sm">{t("label.enabled")}</span>
            </label>

            <div className="space-y-2">
              <Label className="block">{t("settings.subtitleProviderFacets")}</Label>
              <div className="flex flex-wrap gap-3">
                {SUBTITLE_FACETS.map((facet) => (
                  <label
                    key={facet.value}
                    className="flex items-center gap-2 rounded-lg border border-border/60 bg-card/40 px-3 py-2 text-sm"
                  >
                    <input
                      id={selectorId("settings-subtitle-provider-facet", facet.value)}
                      type="checkbox"
                      checked={providerDraft.enabledFacets.includes(facet.value)}
                      onChange={(event) =>
                        handleFacetToggle(facet.value, event.target.checked)
                      }
                      className="accent-primary"
                    />
                    <span>{t(facet.labelKey)}</span>
                  </label>
                ))}
              </div>
              <p className="text-xs text-muted-foreground">
                {t("settings.subtitleProviderFacetsHelp")}
              </p>
            </div>

            {selectedProviderFields.length > 0 ? (
              <div className="grid gap-4 md:grid-cols-2">
                {selectedProviderFields.map((field) => (
                  <DynamicSubtitleConfigField
                    key={`${normalizedProviderType}:${field.key}`}
                    field={field}
                    value={providerDraft.configValues[field.key] ?? ""}
                    onChange={handleConfigValueChange}
                    hasStoredSecretValue={providerDraft.storedSecretKeys.includes(
                      field.key,
                    )}
                  />
                ))}
              </div>
            ) : normalizedProviderType ? (
              <div className="rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-900 dark:text-amber-100">
                <div className="flex items-start gap-2">
                  <CircleAlert className="mt-0.5 h-4 w-4 shrink-0 text-amber-500" />
                  <p>{t("settings.subtitleProviderUnknownType")}</p>
                </div>
              </div>
            ) : null}

            <div className="flex flex-wrap gap-2">
              <Button
                id="settings-subtitle-provider-save"
                type="submit"
                disabled={
                  mutatingProviderId !== null ||
                  !providerDraft.name.trim() ||
                  !normalizedProviderType
                }
              >
                {editingProviderId ? t("label.save") : t("label.create")}
              </Button>
              <Button
                id="settings-subtitle-provider-test-connection"
                type="button"
                variant="secondary"
                onClick={() => void testProviderConnection()}
                disabled={isTestingConnection || !normalizedProviderType}
              >
                {isTestingConnection ? (
                  <Loader2 className="mr-1 h-4 w-4 animate-spin" />
                ) : null}
                {t("label.testConnection")}
              </Button>
              <Button
                id="settings-subtitle-provider-cancel"
                type="button"
                variant="outline"
                onClick={resetProviderDraft}
                disabled={mutatingProviderId !== null}
              >
                {t("label.cancel")}
              </Button>
            </div>
          </form>
          {isEditing ? (
            <div className="flex justify-center">
              <Button
                id="settings-subtitle-provider-create-new"
                type="button"
                size="lg"
                onClick={startCreateProvider}
                disabled={mutatingProviderId !== null}
                className="h-12 border border-emerald-500/30 bg-emerald-500/15 px-5 text-base font-semibold text-emerald-100 hover:bg-emerald-500/25 hover:text-emerald-50"
              >
                <Plus className="h-5 w-5" />
                {t("settings.subtitleProviderCreateNew")}
              </Button>
            </div>
          ) : null}
        </>
      ) : (
        <div className="flex justify-center">
          <Button
            id="settings-subtitle-provider-create"
            type="button"
            size="lg"
            onClick={startCreateProvider}
            className="h-12 border border-emerald-500/30 bg-emerald-500/15 px-5 text-base font-semibold text-emerald-100 hover:bg-emerald-500/25 hover:text-emerald-50"
          >
            <Plus className="h-5 w-5" />
            {t("settings.subtitleProviderCreateNew")}
          </Button>
        </div>
      )}
    </div>
  );
}
