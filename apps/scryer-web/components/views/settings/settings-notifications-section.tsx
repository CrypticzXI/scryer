
import * as React from "react";
import { Bell, ChevronDown, Edit, Loader2, Power, PowerOff, Send, Trash2 } from "lucide-react";
import { Link } from "react-router-dom";
import { InfoHelp } from "@/components/common/info-help";
import { TitleAutocompletePicker } from "@/components/common/title-autocomplete-picker";
import { LocalRemotePathMappingsField } from "@/components/common/local-remote-path-mappings-field";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input, signedIntegerInputProps } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
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
import type { Translate } from "@/components/root/types";
import { useTranslate } from "@/lib/context/translate-context";
import { sectionLabelForFacet } from "@/lib/facets/helpers";
import { viewFromFacet } from "@/lib/facets/helpers";
import type {
  ConfigFieldDef,
  NotificationChannel,
  NotificationChannelDraft,
  NotificationProviderType,
  NotificationSubscriptionDraft,
  NotificationSubscriptionRow,
  TitleRecord,
} from "@/lib/types";
import type { Facet } from "@/lib/types/titles";
import { buildOverviewDetailPath } from "@/lib/utils/routing";

type SettingsNotificationsSectionProps = {
  channels: NotificationChannel[];
  editingChannelId: string | null;
  channelDraft: NotificationChannelDraft;
  setChannelDraft: React.Dispatch<React.SetStateAction<NotificationChannelDraft>>;
  submitChannel: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
  mutatingChannelId: string | null;
  resetChannelDraft: () => void;
  editChannel: (channel: NotificationChannel) => void;
  toggleChannelEnabled: (channel: NotificationChannel) => Promise<void> | void;
  deleteChannel: (channel: NotificationChannel) => Promise<void> | void;
  testChannel: (channel: NotificationChannel) => Promise<void> | void;
  testingChannelId: string | null;
  providerTypes: NotificationProviderType[];
  subscriptions: NotificationSubscriptionRow[];
  subscriptionTitlesById: Record<string, TitleRecord | null>;
  editingSubscriptionId: string | null;
  subscriptionDraft: NotificationSubscriptionDraft;
  setSubscriptionDraft: React.Dispatch<React.SetStateAction<NotificationSubscriptionDraft>>;
  submitSubscription: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
  mutatingSubscriptionId: string | null;
  resetSubscriptionDraft: () => void;
  editSubscription: (sub: NotificationSubscriptionRow) => void;
  toggleSubscriptionEnabled: (sub: NotificationSubscriptionRow) => Promise<void> | void;
  deleteSubscription: (sub: NotificationSubscriptionRow) => Promise<void> | void;
  eventTypes: string[];
};

const SCOPE_OPTIONS = ["global", "facet", "title"] as const;
const FACET_SCOPE_OPTIONS: Facet[] = ["movie", "series", "anime"];

const NOTIFICATION_EVENT_LABEL_KEYS: Record<string, string> = {
  grab: "settings.notificationEvent.grab",
  download: "settings.notificationEvent.download",
  upgrade: "settings.notificationEvent.upgrade",
  import_complete: "settings.notificationEvent.importComplete",
  import_rejected: "settings.notificationEvent.importRejected",
  rename: "settings.notificationEvent.rename",
  title_added: "settings.notificationEvent.titleAdded",
  title_deleted: "settings.notificationEvent.titleDeleted",
  file_deleted: "settings.notificationEvent.fileDeleted",
  file_deleted_for_upgrade: "settings.notificationEvent.fileDeletedForUpgrade",
  post_processing_completed: "settings.notificationEvent.postProcessingCompleted",
  subtitle_downloaded: "settings.notificationEvent.subtitleDownloaded",
  subtitle_search_failed: "settings.notificationEvent.subtitleSearchFailed",
  health_issue: "settings.notificationEvent.healthIssue",
  health_restored: "settings.notificationEvent.healthRestored",
  application_update: "settings.notificationEvent.applicationUpdate",
  manual_interaction_required: "settings.notificationEvent.manualInteractionRequired",
  test: "settings.notificationEvent.test",
};

function humanizeSnakeCase(value: string) {
  return value
    .split("_")
    .filter(Boolean)
    .map((segment) => segment.charAt(0).toUpperCase() + segment.slice(1))
    .join(" ");
}

function notificationEventLabel(eventType: string, t: Translate) {
  const key = NOTIFICATION_EVENT_LABEL_KEYS[eventType];
  return key ? t(key) : humanizeSnakeCase(eventType);
}

function notificationScopeLabel(scope: string, t: Translate) {
  const key = `settings.notificationScope.${scope}`;
  const translated = t(key);
  return translated === key ? humanizeSnakeCase(scope) : translated;
}

function isFacetScopeValue(value: string): value is Facet {
  return FACET_SCOPE_OPTIONS.includes(value as Facet);
}

function parseFacetScopeIds(scopeId: string | null | undefined): Facet[] {
  if (!scopeId) return [];

  const selected = new Set<Facet>();
  for (const token of scopeId.split(",")) {
    const normalized = token.trim().toLowerCase();
    if (isFacetScopeValue(normalized)) {
      selected.add(normalized);
    }
  }

  return FACET_SCOPE_OPTIONS.filter((facet) => selected.has(facet));
}

function formatResolvedTitleLabel(title: TitleRecord): string {
  return title.year ? `${title.name} (${title.year})` : title.name;
}

function titleOverviewHref(title: TitleRecord): string {
  const titleSlug = title.slug?.trim() || null;
  const librarySlug = title.librarySlug?.trim() || null;
  const basePath = buildOverviewDetailPath(viewFromFacet(title.facet), librarySlug, titleSlug);
  if (titleSlug && librarySlug) {
    return basePath;
  }
  return `${basePath}?id=${encodeURIComponent(title.id)}`;
}

function notificationScopeIdLabel(
  scope: string,
  scopeId: string | null | undefined,
  t: Translate,
  subscriptionTitlesById: Record<string, TitleRecord | null>,
): string | null {
  if (!scopeId) {
    return null;
  }

  if (scope === "title") {
    const title = subscriptionTitlesById[scopeId];
    return title ? formatResolvedTitleLabel(title) : t("label.unknown");
  }

  if (scope !== "facet") {
    return scopeId;
  }

  const facets = parseFacetScopeIds(scopeId);
  if (facets.length === 0) {
    return scopeId;
  }

  return facets.map((facet) => sectionLabelForFacet(t, facet)).join(", ");
}

function DynamicConfigField({
  field,
  value,
  onChange,
}: {
  field: ConfigFieldDef;
  value: string;
  onChange: (key: string, value: string) => void;
}) {
  const help = field.helpText ? (
    <InfoHelp
      text={field.helpText}
      ariaLabel={`About ${field.label}`}
    />
  ) : null;
  const requiredMarker = field.required ? (
    <span aria-hidden="true" className="text-destructive">
      *
    </span>
  ) : null;

  if (field.fieldType === "bool") {
    return (
      <label className="flex items-center gap-2">
        <input
          type="checkbox"
          checked={value === "true"}
          onChange={(e) => onChange(field.key, e.target.checked ? "true" : "false")}
          className="accent-primary"
        />
        <span className="inline-flex items-center gap-2 text-sm">
          {field.label}
          {requiredMarker}
          {help}
        </span>
      </label>
    );
  }

  if (field.fieldType === "select" && field.options.length > 0) {
    return (
      <label>
        <Label className="mb-2 inline-flex items-center gap-2">
          {field.label}
          {requiredMarker}
          {help}
        </Label>
        <Select
          value={value || field.defaultValue || ""}
          onValueChange={(v) => onChange(field.key, v)}
        >
          <SelectTrigger className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {field.options.map((opt) => (
              <SelectItem key={opt.value} value={opt.value}>{opt.label}</SelectItem>
            ))}
          </SelectContent>
        </Select>
      </label>
    );
  }

  if (field.fieldType === "multiline") {
    if (field.key === "path_mappings") {
      return (
        <LocalRemotePathMappingsField
          fieldKey={field.key}
          label={field.label}
          value={value}
          helpText={field.helpText}
          required={field.required}
          onChange={onChange}
        />
      );
    }

    return (
      <label>
        <Label className="mb-2 inline-flex items-center gap-2">
          {field.label}
          {requiredMarker}
          {help}
        </Label>
        <Textarea
          value={value}
          onChange={(e) => onChange(field.key, e.target.value)}
          required={field.required}
          placeholder={field.defaultValue ?? ""}
          rows={6}
        />
      </label>
    );
  }

  return (
    <label>
      <Label className="mb-2 inline-flex items-center gap-2">
        {field.label}
        {requiredMarker}
        {help}
      </Label>
      <Input
        value={value}
        onChange={(e) => onChange(field.key, e.target.value)}
        {...(field.fieldType === "number" ? signedIntegerInputProps : {})}
        type={
          field.fieldType === "password" || field.fieldType === "secret"
            ? "password"
            : field.fieldType === "number"
              ? "number"
              : "text"
        }
        required={field.required}
        placeholder={field.defaultValue ?? ""}
      />
    </label>
  );
}

function channelNameById(channels: NotificationChannel[], id: string): string {
  return channels.find((c) => c.id === id)?.name ?? id;
}

type MultiSelectDropdownOption = {
  value: string;
  label: string;
};

function MultiSelectDropdown({
  options,
  selectedValues,
  onSelectedValuesChange,
  placeholder,
}: {
  options: MultiSelectDropdownOption[];
  selectedValues: string[];
  onSelectedValuesChange: (values: string[]) => void;
  placeholder: string;
}) {
  const selectedLabel = React.useMemo(() => {
    const labels = options
      .filter((option) => selectedValues.includes(option.value))
      .map((option) => option.label);
    return labels.length > 0 ? labels.join(", ") : placeholder;
  }, [options, placeholder, selectedValues]);

  const toggleOption = React.useCallback(
    (value: string) => {
      const selectedSet = new Set(selectedValues);
      if (selectedSet.has(value)) {
        selectedSet.delete(value);
      } else {
        selectedSet.add(value);
      }

      onSelectedValuesChange(
        options
          .map((option) => option.value)
          .filter((optionValue) => selectedSet.has(optionValue)),
      );
    },
    [onSelectedValuesChange, options, selectedValues],
  );

  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button
          type="button"
          variant="outline"
          className="w-full justify-between px-3 text-left font-normal"
        >
          <span
            className={`truncate ${selectedValues.length === 0 ? "text-muted-foreground" : ""}`}
          >
            {selectedLabel}
          </span>
          <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground" />
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-[var(--radix-popover-trigger-width)] p-2">
        <div className="flex max-h-72 flex-col gap-1 overflow-y-auto">
          {options.map((option) => {
            const checked = selectedValues.includes(option.value);
            return (
              <button
                key={option.value}
                type="button"
                onClick={() => toggleOption(option.value)}
                className="flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm transition-colors hover:bg-accent"
              >
                <Checkbox checked={checked} className="pointer-events-none" />
                <span className="truncate">{option.label}</span>
              </button>
            );
          })}
        </div>
      </PopoverContent>
    </Popover>
  );
}

export function SettingsNotificationsSection({
  channels,
  editingChannelId,
  channelDraft,
  setChannelDraft,
  submitChannel,
  mutatingChannelId,
  resetChannelDraft,
  editChannel,
  toggleChannelEnabled,
  deleteChannel,
  testChannel,
  testingChannelId,
  providerTypes,
  subscriptions,
  subscriptionTitlesById,
  editingSubscriptionId,
  subscriptionDraft,
  setSubscriptionDraft,
  submitSubscription,
  mutatingSubscriptionId,
  resetSubscriptionDraft,
  editSubscription,
  toggleSubscriptionEnabled,
  deleteSubscription,
  eventTypes,
}: SettingsNotificationsSectionProps) {
  const t = useTranslate();
  const normalizedChannelType = channelDraft.channelType.trim().toLowerCase();
  const selectedFacetScopeIds = subscriptionDraft.facetScopeIds;
  const scopeOptions = React.useMemo(
    () =>
      SCOPE_OPTIONS.map((scope) => ({
        value: scope,
        label: notificationScopeLabel(scope, t),
      })),
    [t],
  );
  const eventTypeOptions = React.useMemo(
    () =>
      eventTypes.map((eventType) => ({
        value: eventType,
        label: notificationEventLabel(eventType, t),
      })),
    [eventTypes, t],
  );
  const orderedSubscriptionEventTypes = React.useCallback(
    (values: string[]) => {
      const order = new Map(eventTypes.map((value, index) => [value, index]));
      return [...values].sort((left, right) => {
        const leftIndex = order.get(left) ?? Number.MAX_SAFE_INTEGER;
        const rightIndex = order.get(right) ?? Number.MAX_SAFE_INTEGER;
        if (leftIndex !== rightIndex) {
          return leftIndex - rightIndex;
        }
        return left.localeCompare(right);
      });
    },
    [eventTypes],
  );
  const isSubscriptionDraftValid =
    subscriptionDraft.channelId.trim().length > 0 &&
    subscriptionDraft.eventTypes.length > 0 &&
    subscriptionDraft.scope.trim().length > 0 &&
    (subscriptionDraft.scope !== "facet" || selectedFacetScopeIds.length > 0) &&
    (subscriptionDraft.scope !== "title" ||
      subscriptionDraft.titleScopeId.trim().length > 0);

  const providerTypeOptions = React.useMemo(() => {
    if (providerTypes.length === 0) return [];
    return providerTypes.map((pt) => ({ value: pt.providerType, label: pt.name }));
  }, [providerTypes]);

  const selectedProvider = React.useMemo(() => {
    return providerTypes.find(
      (pt) => pt.providerType === normalizedChannelType,
    ) ?? null;
  }, [normalizedChannelType, providerTypes]);

  const selectedProviderFields = selectedProvider?.configFields ?? [];

  const handleConfigValueChange = React.useCallback(
    (key: string, value: string) => {
      setChannelDraft((prev) => ({
        ...prev,
        configValues: { ...prev.configValues, [key]: value },
      }));
    },
    [setChannelDraft],
  );

  const handleFacetScopeCheckedChange = React.useCallback(
    (facet: Facet, checked: boolean) => {
      setSubscriptionDraft((prev) => {
        const next = new Set(prev.facetScopeIds);
        if (checked) {
          next.add(facet);
        } else {
          next.delete(facet);
        }
        return {
          ...prev,
          facetScopeIds: FACET_SCOPE_OPTIONS.filter((scopeOption) => next.has(scopeOption)),
        };
      });
    },
    [setSubscriptionDraft],
  );

  return (
    <div className="space-y-6 text-sm">
      {/* ── Channels ──────────────────────────────────── */}
      <CardTitle className="flex items-center gap-2 text-base">
        <Bell className="h-4 w-4" />
        {t("settings.notificationChannels")}
      </CardTitle>

      <div className="rounded border border-border">
        <div className="overflow-x-auto">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("label.name")}</TableHead>
                <TableHead>{t("settings.notificationProviderType")}</TableHead>
                <TableHead className="text-center">{t("label.enabled")}</TableHead>
                <TableHead className="text-right">{t("label.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {channels.map((channel) => (
                <TableRow key={channel.id}>
                  <TableCell>{channel.name}</TableCell>
                  <TableCell>{channel.channelType}</TableCell>
                  <TableCell className="text-center">
                    <RenderBooleanIcon
                      value={channel.isEnabled}
                      label={`${t("label.enabled")}: ${channel.name}`}
                    />
                  </TableCell>
                  <TableCell className="text-right">
                    <div className="flex justify-end gap-2">
                      <Button
                        size="sm"
                        variant="outline"
                        onClick={() => void testChannel(channel)}
                        disabled={testingChannelId === channel.id}
                      >
                        {testingChannelId === channel.id ? (
                          <Loader2 className="mr-1 h-3.5 w-3.5 animate-spin" />
                        ) : (
                          <Send className="mr-1 h-3.5 w-3.5" />
                        )}
                        {t("settings.notificationTest")}
                      </Button>
                      <Button
                        size="icon"
                        variant="ghost"
                        onClick={() => void toggleChannelEnabled(channel)}
                        disabled={mutatingChannelId === channel.id}
                        title={channel.isEnabled ? t("label.disable") : t("label.enable")}
                      >
                        {channel.isEnabled ? (
                          <Power className="h-4 w-4 text-green-400" />
                        ) : (
                          <PowerOff className="h-4 w-4 text-red-400" />
                        )}
                      </Button>
                      <Button
                        size="sm"
                        variant="secondary"
                        onClick={() => editChannel(channel)}
                      >
                        <Edit className="mr-1 h-3.5 w-3.5" />
                        {t("label.update")}
                      </Button>
                      <Button
                        size="sm"
                        variant="destructive"
                        onClick={() => void deleteChannel(channel)}
                        disabled={mutatingChannelId === channel.id}
                      >
                        {mutatingChannelId === channel.id ? (
                          <Loader2 className="mr-1 h-3.5 w-3.5 animate-spin" />
                        ) : (
                          <Trash2 className="mr-1 h-3.5 w-3.5" />
                        )}
                        {t("label.delete")}
                      </Button>
                    </div>
                  </TableCell>
                </TableRow>
              ))}
              {channels.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={4} className="text-muted-foreground">
                    {t("settings.notificationNoChannels")}
                  </TableCell>
                </TableRow>
              ) : null}
            </TableBody>
          </Table>
        </div>
      </div>

      <Card>
        <CardHeader>
          <CardTitle className="text-base">
            {editingChannelId ? t("settings.notificationChannelUpdate") : t("settings.notificationChannelCreate")}
          </CardTitle>
        </CardHeader>
        <CardContent>
          {providerTypeOptions.length === 0 ? (
            <p className="text-sm text-muted-foreground">{t("settings.notificationNoProviders")}</p>
          ) : (
            <form className="space-y-3" onSubmit={submitChannel}>
               <div className="grid gap-3 md:grid-cols-2">
                 <label>
                   <Label className="mb-2 block">{t("settings.notificationProviderType")}</Label>
                   <Select
                     value={normalizedChannelType || undefined}
                     onValueChange={(v) => {
                       const provider = providerTypes.find((pt) => pt.providerType === v);
                       setChannelDraft((prev) => ({
                         ...prev,
                         channelType: v,
                         name: provider?.name ?? "",
                       }));
                     }}
                   >
                     <SelectTrigger className="w-full">
                       <SelectValue placeholder={t("settings.notificationProviderType")} />
                     </SelectTrigger>
                     <SelectContent>
                       {providerTypeOptions.map((opt) => (
                         <SelectItem key={opt.value} value={opt.value}>{opt.label}</SelectItem>
                       ))}
                     </SelectContent>
                   </Select>
                 </label>
                 <label>
                   <Label className="mb-2 block">{t("label.name")}</Label>
                   <Input
                     value={channelDraft.name}
                     onChange={(event) =>
                       setChannelDraft((prev) => ({
                         ...prev,
                         name: event.target.value,
                       }))
                     }
                     required
                     placeholder="My Webhook"
                   />
                 </label>
               </div>

              {selectedProviderFields.length > 0 ? (
                <div className="space-y-3">
                  <div className="grid gap-3 md:grid-cols-2">
                    {selectedProviderFields
                      .filter((f) => f.fieldType !== "bool")
                      .map((field) => (
                        <DynamicConfigField
                          key={field.key}
                          field={field}
                          value={channelDraft.configValues[field.key] ?? field.defaultValue ?? ""}
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
                            value={channelDraft.configValues[field.key] ?? field.defaultValue ?? "false"}
                            onChange={handleConfigValueChange}
                          />
                        ))}
                    </div>
                  ) : null}
                </div>
              ) : null}

              <label className="flex items-center gap-2">
                <input
                  type="checkbox"
                  checked={channelDraft.isEnabled}
                  onChange={(event) =>
                    setChannelDraft((prev) => ({
                      ...prev,
                      isEnabled: event.target.checked,
                    }))
                  }
                  className="accent-primary"
                />
                <span className="text-sm">{t("label.enabled")}</span>
              </label>

              <div className="flex gap-2">
                <Button type="submit" disabled={mutatingChannelId === "new"}>
                  {mutatingChannelId === "new"
                    ? t("label.saving")
                    : editingChannelId
                      ? t("settings.notificationChannelUpdate")
                      : t("settings.notificationChannelCreate")}
                </Button>
                <Button type="button" variant="secondary" onClick={resetChannelDraft}>
                  {t("label.cancel")}
                </Button>
              </div>
            </form>
          )}
        </CardContent>
      </Card>

      {/* ── Subscriptions ─────────────────────────────── */}
      <CardTitle className="flex items-center gap-2 text-base">
        <Bell className="h-4 w-4" />
        {t("settings.notificationSubscriptions")}
      </CardTitle>

      <div className="rounded border border-border">
        <div className="overflow-x-auto">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("settings.notificationEventType")}</TableHead>
                <TableHead>{t("settings.notificationChannel")}</TableHead>
                <TableHead>{t("settings.notificationScope")}</TableHead>
                <TableHead className="text-center">{t("label.enabled")}</TableHead>
                <TableHead className="text-right">{t("label.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {subscriptions.map((sub) => (
                <TableRow key={sub.id}>
                  <TableCell>
                    <div className="space-y-1">
                      {orderedSubscriptionEventTypes(sub.eventTypes).map((eventType) => (
                        <div key={eventType} className="leading-snug">
                          {notificationEventLabel(eventType, t)}
                        </div>
                      ))}
                    </div>
                  </TableCell>
                  <TableCell>{channelNameById(channels, sub.channelId)}</TableCell>
                  <TableCell>
                    {(() => {
                      const scopeIdLabel = notificationScopeIdLabel(
                        sub.scope,
                        sub.scopeId,
                        t,
                        subscriptionTitlesById,
                      );

                      if (sub.scope !== "title" || !sub.scopeId) {
                        return (
                          <>
                            {notificationScopeLabel(sub.scope, t)}
                            {scopeIdLabel ? ` (${scopeIdLabel})` : ""}
                          </>
                        );
                      }

                      const title = subscriptionTitlesById[sub.scopeId];

                      return (
                        <>
                          {notificationScopeLabel(sub.scope, t)}
                          {" ("}
                          {title ? (
                            <Link
                              to={titleOverviewHref(title)}
                              className="underline-offset-4 hover:text-foreground hover:underline"
                            >
                              {formatResolvedTitleLabel(title)}
                            </Link>
                          ) : (
                            scopeIdLabel ?? t("label.unknown")
                          )}
                          {")"}
                        </>
                      );
                    })()}
                  </TableCell>
                  <TableCell className="text-center">
                    <RenderBooleanIcon
                      value={sub.isEnabled}
                      label={`${t("label.enabled")}: ${orderedSubscriptionEventTypes(sub.eventTypes)
                        .map((eventType) => notificationEventLabel(eventType, t))
                        .join(", ")}`}
                    />
                  </TableCell>
                  <TableCell className="text-right">
                    <div className="flex justify-end gap-2">
                      <Button
                        size="icon"
                        variant="ghost"
                        onClick={() => void toggleSubscriptionEnabled(sub)}
                        disabled={mutatingSubscriptionId === sub.id}
                        title={sub.isEnabled ? t("label.disable") : t("label.enable")}
                      >
                        {sub.isEnabled ? (
                          <Power className="h-4 w-4 text-green-400" />
                        ) : (
                          <PowerOff className="h-4 w-4 text-red-400" />
                        )}
                      </Button>
                      <Button
                        size="sm"
                        variant="secondary"
                        onClick={() => editSubscription(sub)}
                      >
                        <Edit className="mr-1 h-3.5 w-3.5" />
                        {t("label.update")}
                      </Button>
                      <Button
                        size="sm"
                        variant="destructive"
                        onClick={() => void deleteSubscription(sub)}
                        disabled={mutatingSubscriptionId === sub.id}
                      >
                        {mutatingSubscriptionId === sub.id ? (
                          <Loader2 className="mr-1 h-3.5 w-3.5 animate-spin" />
                        ) : (
                          <Trash2 className="mr-1 h-3.5 w-3.5" />
                        )}
                        {t("label.delete")}
                      </Button>
                    </div>
                  </TableCell>
                </TableRow>
              ))}
              {subscriptions.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={5} className="text-muted-foreground">
                    {t("settings.notificationNoSubscriptions")}
                  </TableCell>
                </TableRow>
              ) : null}
            </TableBody>
          </Table>
        </div>
      </div>

      <Card>
        <CardHeader>
          <CardTitle className="text-base">
            {editingSubscriptionId ? t("settings.notificationSubscriptionUpdate") : t("settings.notificationSubscriptionCreate")}
          </CardTitle>
        </CardHeader>
        <CardContent>
          {channels.length === 0 ? (
            <p className="text-sm text-muted-foreground">{t("settings.notificationNoChannels")}</p>
          ) : (
            <form className="space-y-3" onSubmit={submitSubscription}>
              <div className="grid gap-3 md:grid-cols-3">
                <label>
                  <Label className="mb-2 block">{t("settings.notificationEventType")}</Label>
                  <MultiSelectDropdown
                    options={eventTypeOptions}
                    selectedValues={subscriptionDraft.eventTypes}
                    onSelectedValuesChange={(values) =>
                      setSubscriptionDraft((prev) => ({
                        ...prev,
                        eventTypes: values,
                      }))
                    }
                    placeholder={t("settings.notificationEventType")}
                  />
                </label>
                <label>
                  <Label className="mb-2 block">{t("settings.notificationChannel")}</Label>
                  <Select
                    value={subscriptionDraft.channelId || undefined}
                    onValueChange={(v) =>
                      setSubscriptionDraft((prev) => ({
                        ...prev,
                        channelId: v,
                      }))
                    }
                  >
                    <SelectTrigger className="w-full">
                      <SelectValue placeholder={t("settings.notificationChannel")} />
                    </SelectTrigger>
                    <SelectContent>
                      {channels.map((ch) => (
                        <SelectItem key={ch.id} value={ch.id}>{ch.name}</SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </label>
                <label>
                  <Label className="mb-2 block">{t("settings.notificationScope")}</Label>
                  <Select
                    value={subscriptionDraft.scope || undefined}
                    onValueChange={(value) =>
                      setSubscriptionDraft((prev) => ({
                        ...prev,
                        scope: value,
                      }))
                    }
                  >
                    <SelectTrigger className="w-full">
                      <SelectValue placeholder={t("settings.notificationScope")} />
                    </SelectTrigger>
                    <SelectContent>
                      {scopeOptions.map((option) => (
                        <SelectItem key={option.value} value={option.value}>
                          {option.label}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </label>
              </div>

              {subscriptionDraft.scope === "facet" ? (
                <div>
                  <Label className="mb-2 block">{t("settings.notificationScopeId")}</Label>
                  <div className="flex flex-wrap gap-4 rounded-md border border-border bg-card/40 p-3">
                    {FACET_SCOPE_OPTIONS.map((facet) => (
                      <label key={facet} className="flex items-center gap-2 text-sm">
                        <Checkbox
                          checked={selectedFacetScopeIds.includes(facet)}
                          onCheckedChange={(checked) =>
                            handleFacetScopeCheckedChange(facet, checked === true)
                          }
                        />
                        {sectionLabelForFacet(t, facet)}
                      </label>
                    ))}
                  </div>
                </div>
              ) : null}

              {subscriptionDraft.scope === "title" ? (
                <label>
                  <Label className="mb-2 block">{t("settings.notificationScopeId")}</Label>
                  <TitleAutocompletePicker
                    selectedTitle={subscriptionDraft.titleScopeTitle}
                    selectedTitleId={subscriptionDraft.titleScopeId || null}
                    onSelectedTitleChange={(title) =>
                      setSubscriptionDraft((prev) => ({
                        ...prev,
                        titleScopeId: title?.id ?? "",
                        titleScopeTitle: title,
                      }))
                    }
                    placeholder={t("settings.notificationScopeIdPlaceholderTitle")}
                    ariaLabel={t("settings.notificationScopeId")}
                  />
                </label>
              ) : null}

              <label className="flex items-center gap-2">
                <input
                  type="checkbox"
                  checked={subscriptionDraft.isEnabled}
                  onChange={(event) =>
                    setSubscriptionDraft((prev) => ({
                      ...prev,
                      isEnabled: event.target.checked,
                    }))
                  }
                  className="accent-primary"
                />
                <span className="text-sm">{t("label.enabled")}</span>
              </label>

              <div className="flex gap-2">
                <Button type="submit" disabled={!isSubscriptionDraftValid || mutatingSubscriptionId !== null}>
                  {mutatingSubscriptionId !== null
                    ? t("label.saving")
                    : editingSubscriptionId
                      ? t("settings.notificationSubscriptionUpdate")
                      : t("settings.notificationSubscriptionCreate")}
                </Button>
                <Button type="button" variant="secondary" onClick={resetSubscriptionDraft}>
                  {t("label.cancel")}
                </Button>
              </div>
            </form>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
