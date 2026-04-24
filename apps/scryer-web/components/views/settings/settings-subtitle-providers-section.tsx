import * as React from "react";
import {
  CircleAlert,
  Edit,
  Loader2,
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

  if (field.fieldType === "bool") {
    return (
      <label className="flex items-center gap-2 rounded-lg border border-border/60 bg-card/40 px-3 py-2">
        <input
          type="checkbox"
          checked={value === "true"}
          onChange={(event) =>
            onChange(field.key, event.target.checked ? "true" : "false")
          }
          className="accent-primary"
        />
        <div className="space-y-1">
          <span className="text-sm font-medium">{field.label}</span>
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
        <Label className="mb-2 block">{field.label}</Label>
        <Select
          value={value || field.defaultValue || ""}
          onValueChange={(nextValue) => onChange(field.key, nextValue)}
        >
          <SelectTrigger className="w-full">
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
        <Label className="mb-2 block">{field.label}</Label>
        <Textarea
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
      <Label className="mb-2 block">{field.label}</Label>
      <Input
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
}: Props) {
  const t = useTranslate();
  const normalizedProviderType = providerDraft.providerType.trim().toLowerCase();

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
          name:
            previous.name.trim().length > 0
              ? previous.name
              : (nextProvider?.name ?? previous.name),
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
    <div className="space-y-4 rounded-xl border border-border/60 bg-card/30 p-4">
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
                  <TableRow key={provider.id}>
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
                          label={t("label.edit")}
                          tone="edit"
                          onClick={() => editProvider(provider)}
                          disabled={mutatingProviderId !== null}
                        >
                          <Edit className="h-4 w-4" />
                        </SubtitleProviderActionButton>
                        <SubtitleProviderActionButton
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

      <form className="space-y-4 rounded-lg border border-border/60 p-4" onSubmit={submitProvider}>
        <div className="flex items-center justify-between gap-3">
          <CardTitle className="text-base">
            {editingProviderId
              ? t("settings.subtitleProviderEdit")
              : t("settings.subtitleProviderCreate")}
          </CardTitle>
          {mutatingProviderId ? (
            <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
          ) : null}
        </div>

        <div className="grid gap-4 md:grid-cols-2">
          <label>
            <Label className="mb-2 block">{t("label.name")}</Label>
            <Input
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

          <label>
            <Label className="mb-2 block">{t("settings.subtitleProviderType")}</Label>
            <Select
              value={normalizedProviderType}
              onValueChange={handleProviderTypeChange}
            >
              <SelectTrigger className="w-full">
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
        </div>

        <label className="flex items-center gap-2">
          <input
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
                hasStoredSecretValue={providerDraft.storedSecretKeys.includes(field.key)}
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
            type="button"
            variant="secondary"
            onClick={() => void testProviderConnection()}
            disabled={
              isTestingConnection ||
              !normalizedProviderType
            }
          >
            {isTestingConnection ? (
              <Loader2 className="mr-1 h-4 w-4 animate-spin" />
            ) : null}
            {t("label.testConnection")}
          </Button>
          {editingProviderId ? (
            <Button
              type="button"
              variant="ghost"
              onClick={resetProviderDraft}
              disabled={mutatingProviderId !== null}
            >
              {t("label.cancel")}
            </Button>
          ) : null}
        </div>
      </form>
    </div>
  );
}
