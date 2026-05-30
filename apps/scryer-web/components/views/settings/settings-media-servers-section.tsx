import * as React from "react";
import {
  CheckCircle2,
  Edit,
  KeyRound,
  Loader2,
  Plus,
  Power,
  PowerOff,
  Server,
  ShieldCheck,
  Trash2,
} from "lucide-react";
import { LocalRemotePathMappingsField } from "@/components/common/local-remote-path-mappings-field";
import { RenderBooleanIcon } from "@/components/common/boolean-icon";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useTranslate } from "@/lib/context/translate-context";
import type {
  LibraryRecord,
  MediaServerConnection,
  MediaServerConnectionDraft,
  MediaServerDefaultLibraryGrant,
  MediaServerProvider,
} from "@/lib/types";
import { cn } from "@/lib/utils";
import { selectorId } from "@/lib/utils/dom-ids";
import {
  APP_PERMISSIONS,
  LIBRARY_PERMISSIONS,
  type AppPermission,
  type LibraryPermission,
} from "@/lib/utils/permissions";
import {
  boxedActionButtonBaseClass,
  boxedActionButtonToneClass,
  type BoxedActionButtonTone,
} from "@/lib/utils/action-button-styles";

type SettingsMediaServersSectionProps = {
  connections: MediaServerConnection[];
  libraries: LibraryRecord[];
  draft: MediaServerConnectionDraft;
  setDraft: React.Dispatch<React.SetStateAction<MediaServerConnectionDraft>>;
  editingConnectionId: string | null;
  mutatingConnectionId: string | null;
  testingConnectionId: string | null;
  isEditorOpen: boolean;
  editorMode: "create" | "edit";
  submitConnection: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
  editConnection: (connection: MediaServerConnection) => void;
  testConnection: (connection: MediaServerConnection) => Promise<void> | void;
  toggleConnectionEnabled: (connection: MediaServerConnection) => Promise<void> | void;
  deleteConnection: (connection: MediaServerConnection) => Promise<void> | void;
  resetDraft: () => void;
  startCreateConnection: () => void;
};

const PROVIDERS: Array<{ value: MediaServerProvider; label: string }> = [
  { value: "jellyfin", label: "Jellyfin" },
  { value: "plex", label: "Plex" },
  { value: "emby", label: "Emby" },
];

const DEFAULT_BASE_URL_BY_PROVIDER: Record<MediaServerProvider, string> = {
  jellyfin: "",
  plex: "https://plex.tv",
  emby: "",
};

const DEFAULT_NAME_BY_PROVIDER: Record<MediaServerProvider, string> = {
  jellyfin: "Jellyfin",
  plex: "Plex",
  emby: "Emby",
};

const APP_PERMISSION_OPTIONS: Array<{ value: AppPermission; label: string }> = [
  { value: APP_PERMISSIONS.manageUsers, label: "Manage users" },
  { value: APP_PERMISSIONS.managePermissions, label: "Manage permissions" },
  { value: APP_PERMISSIONS.manageSystemSettings, label: "Manage system settings" },
  { value: APP_PERMISSIONS.manageCatalogSettings, label: "Manage catalog settings" },
];

const LIBRARY_PERMISSION_OPTIONS: Array<{ value: LibraryPermission; label: string }> = [
  { value: LIBRARY_PERMISSIONS.view, label: "View" },
  { value: LIBRARY_PERMISSIONS.request, label: "Request" },
  { value: LIBRARY_PERMISSIONS.autoApproveRequests, label: "Auto approve requests" },
  { value: LIBRARY_PERMISSIONS.manageTitles, label: "Manage titles" },
  { value: LIBRARY_PERMISSIONS.resolveImports, label: "Resolve imports" },
  { value: LIBRARY_PERMISSIONS.manageLibrary, label: "Manage library" },
];

function providerLabel(provider: MediaServerProvider): string {
  return PROVIDERS.find((candidate) => candidate.value === provider)?.label ?? provider;
}

function providerSupportsAuth(provider: MediaServerProvider): boolean {
  return provider === "jellyfin" || provider === "plex";
}

function toggleListValue<T extends string>(values: T[], value: T): T[] {
  return values.includes(value)
    ? values.filter((candidate) => candidate !== value)
    : [...values, value];
}

function selectedLibraryGrant(
  grants: MediaServerDefaultLibraryGrant[],
  libraryId: string,
): string[] {
  return grants.find((grant) => grant.libraryId === libraryId)?.permissions ?? [];
}

function updateLibraryGrant(
  grants: MediaServerDefaultLibraryGrant[],
  libraryId: string,
  permissions: string[],
): MediaServerDefaultLibraryGrant[] {
  const filtered = grants.filter((grant) => grant.libraryId !== libraryId);
  const normalized = Array.from(new Set(permissions.map((permission) => permission.trim()).filter(Boolean)));
  return normalized.length > 0
    ? [...filtered, { libraryId, permissions: normalized }]
    : filtered;
}

function capabilityBadges(connection: MediaServerConnection): Array<{ label: string; tone: string }> {
  const badges: Array<{ label: string; tone: string }> = [];
  if (connection.provider === "emby") {
    badges.push({
      label: "Notifications",
      tone: "border-sky-500/40 bg-sky-500/10 text-sky-700 dark:text-sky-300",
    });
    return badges;
  }
  if (connection.loginEnabled) {
    badges.push({
      label: "Login",
      tone: "border-emerald-500/40 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
    });
  }
  if (connection.linkingEnabled) {
    badges.push({
      label: "Linking",
      tone: "border-blue-500/40 bg-blue-500/10 text-blue-700 dark:text-blue-300",
    });
  }
  if (connection.autoAddEnabled) {
    badges.push({
      label: "Auto-add",
      tone: "border-violet-500/40 bg-violet-500/10 text-violet-700 dark:text-violet-300",
    });
  }
  return badges.length > 0
    ? badges
    : [{
        label: "Connection only",
        tone: "border-border bg-background text-muted-foreground",
      }];
}

function MediaServerActionButton({
  label,
  tone,
  className,
  children,
  ...props
}: React.ComponentProps<typeof Button> & {
  label: string;
  tone: Extract<BoxedActionButtonTone, "edit" | "enabled" | "disabled" | "delete" | "neutral">;
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

export function SettingsMediaServersSection({
  connections,
  libraries,
  draft,
  setDraft,
  editingConnectionId,
  mutatingConnectionId,
  testingConnectionId,
  isEditorOpen,
  editorMode,
  submitConnection,
  editConnection,
  testConnection,
  toggleConnectionEnabled,
  deleteConnection,
  resetDraft,
  startCreateConnection,
}: SettingsMediaServersSectionProps) {
  const t = useTranslate();
  const isEditing = editorMode === "edit";
  const supportsAuth = providerSupportsAuth(draft.provider);
  const selectedProviderLabel = providerLabel(draft.provider);

  const handleProviderChange = React.useCallback(
    (provider: MediaServerProvider) => {
      setDraft((previous) => {
        const wasAutofilledName =
          previous.displayName.trim().length === 0 ||
          previous.displayName === DEFAULT_NAME_BY_PROVIDER[previous.provider];
        return {
          ...previous,
          provider,
          displayName: wasAutofilledName
            ? DEFAULT_NAME_BY_PROVIDER[provider]
            : previous.displayName,
          baseUrl: previous.baseUrl.trim() || DEFAULT_BASE_URL_BY_PROVIDER[provider],
          loginEnabled: providerSupportsAuth(provider) ? previous.loginEnabled : false,
          linkingEnabled: providerSupportsAuth(provider) ? previous.linkingEnabled : false,
          autoAddEnabled: providerSupportsAuth(provider) ? previous.autoAddEnabled : false,
          defaultAppPermissions: providerSupportsAuth(provider)
            ? previous.defaultAppPermissions
            : [],
          defaultLibraryGrants: providerSupportsAuth(provider)
            ? previous.defaultLibraryGrants
            : [],
          machineId: provider === "plex" ? previous.machineId : "",
        };
      });
    },
    [setDraft],
  );

  return (
    <div id="settings-media-servers-section" className="space-y-4 text-sm">
      <CardTitle className="flex items-center gap-2 text-base">
        <Server className="h-4 w-4" />
        {t("settings.mediaServersSection")}
      </CardTitle>

      <div id="settings-media-servers-table-card" className="rounded border border-border">
        <div className="overflow-x-auto">
          <Table id="settings-media-servers-table">
            <TableHeader>
              <TableRow>
                <TableHead>{t("label.name")}</TableHead>
                <TableHead>{t("settings.provider")}</TableHead>
                <TableHead>{t("settings.baseUrl")}</TableHead>
                <TableHead>{t("label.enabled")}</TableHead>
                <TableHead>{t("settings.capabilities")}</TableHead>
                <TableHead>{t("settings.credentials")}</TableHead>
                <TableHead className="text-right">{t("label.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {connections.map((connection) => (
                <TableRow
                  key={connection.id}
                  id={selectorId("settings-media-server-row", connection.displayName)}
                >
                  <TableCell className="font-medium">{connection.displayName}</TableCell>
                  <TableCell>{providerLabel(connection.provider)}</TableCell>
                  <TableCell className="max-w-72 truncate">{connection.baseUrl || "-"}</TableCell>
                  <TableCell className="text-center">
                    <RenderBooleanIcon
                      value={connection.enabled}
                      label={`${t("label.enabled")}: ${connection.displayName}`}
                    />
                  </TableCell>
                  <TableCell>
                    <div className="flex flex-wrap gap-1.5">
                      {capabilityBadges(connection).map((badge) => (
                        <span
                          key={badge.label}
                          className={`inline-flex rounded-full border px-2 py-0.5 text-xs font-medium ${badge.tone}`}
                        >
                          {badge.label}
                        </span>
                      ))}
                    </div>
                  </TableCell>
                  <TableCell>
                    {connection.provider === "plex" ? (
                      connection.machineId ? (
                        <span className="inline-flex items-center gap-1 text-emerald-700 dark:text-emerald-300">
                          <CheckCircle2 className="h-3.5 w-3.5" />
                          {t("settings.machineIdConfigured")}
                        </span>
                      ) : (
                        <span className="text-muted-foreground">{t("settings.machineIdMissing")}</span>
                      )
                    ) : connection.apiKeyPresent ? (
                      <span className="inline-flex items-center gap-1 text-emerald-700 dark:text-emerald-300">
                        <KeyRound className="h-3.5 w-3.5" />
                        {t("settings.apiKeyConfigured")}
                      </span>
                    ) : (
                      <span className="text-muted-foreground">{t("settings.apiKeyMissing")}</span>
                    )}
                  </TableCell>
                  <TableCell className="text-right">
                    <div className="inline-flex items-center gap-2">
                      <MediaServerActionButton
                        id={selectorId("settings-media-server-test", connection.displayName)}
                        label={t("label.testConnection")}
                        tone="neutral"
                        onClick={() => void testConnection(connection)}
                        disabled={testingConnectionId === connection.id}
                      >
                        {testingConnectionId === connection.id ? (
                          <Loader2 className="h-4 w-4 animate-spin" />
                        ) : (
                          <ShieldCheck className="h-4 w-4" />
                        )}
                      </MediaServerActionButton>
                      <MediaServerActionButton
                        id={selectorId("settings-media-server-toggle", connection.displayName)}
                        label={connection.enabled ? t("label.disable") : t("label.enable")}
                        tone={connection.enabled ? "enabled" : "disabled"}
                        onClick={() => void toggleConnectionEnabled(connection)}
                        disabled={mutatingConnectionId === connection.id}
                      >
                        {connection.enabled ? (
                          <Power className="h-4 w-4" />
                        ) : (
                          <PowerOff className="h-4 w-4" />
                        )}
                      </MediaServerActionButton>
                      <MediaServerActionButton
                        id={selectorId("settings-media-server-edit", connection.displayName)}
                        label={t("label.edit")}
                        tone="edit"
                        onClick={() => editConnection(connection)}
                      >
                        <Edit className="h-4 w-4" />
                      </MediaServerActionButton>
                      <MediaServerActionButton
                        id={selectorId("settings-media-server-delete", connection.displayName)}
                        label={t("label.delete")}
                        tone="delete"
                        onClick={() => void deleteConnection(connection)}
                        disabled={mutatingConnectionId === connection.id}
                      >
                        {mutatingConnectionId === connection.id ? (
                          <Loader2 className="h-4 w-4 animate-spin" />
                        ) : (
                          <Trash2 className="h-4 w-4" />
                        )}
                      </MediaServerActionButton>
                    </div>
                  </TableCell>
                </TableRow>
              ))}
              {connections.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={7} className="text-muted-foreground">
                    {t("settings.noMediaServersFound")}
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
              <CardTitle id="settings-media-server-editor" className="text-base">
                {isEditing
                  ? t("settings.mediaServerUpdate")
                  : t("settings.mediaServerCreate")}
              </CardTitle>
            </CardHeader>
            <CardContent>
              <form
                id="settings-media-server-form"
                className="space-y-4"
                onSubmit={submitConnection}
              >
                <div className="grid gap-3 md:grid-cols-3">
                  <label>
                    <Label className="mb-2 block" htmlFor="settings-media-server-provider">
                      {t("settings.provider")}
                    </Label>
                    <Select
                      value={draft.provider}
                      onValueChange={(value) => handleProviderChange(value as MediaServerProvider)}
                    >
                      <SelectTrigger id="settings-media-server-provider" className="w-full">
                        <SelectValue aria-label={selectedProviderLabel} />
                      </SelectTrigger>
                      <SelectContent>
                        {PROVIDERS.map((provider) => (
                          <SelectItem key={provider.value} value={provider.value}>
                            {provider.label}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </label>
                  <label>
                    <Label className="mb-2 block" htmlFor="settings-media-server-name">
                      {t("label.name")}
                    </Label>
                    <Input
                      id="settings-media-server-name"
                      value={draft.displayName}
                      onChange={(event) =>
                        setDraft((previous) => ({
                          ...previous,
                          displayName: event.target.value,
                        }))
                      }
                      required
                      placeholder={selectedProviderLabel}
                    />
                  </label>
                  <label>
                    <Label className="mb-2 block" htmlFor="settings-media-server-base-url">
                      {t("settings.baseUrl")}
                    </Label>
                    <Input
                      id="settings-media-server-base-url"
                      value={draft.baseUrl}
                      onChange={(event) =>
                        setDraft((previous) => ({
                          ...previous,
                          baseUrl: event.target.value,
                        }))
                      }
                      required={draft.provider !== "plex"}
                      placeholder={
                        draft.provider === "plex"
                          ? "https://plex.tv"
                          : `https://${draft.provider}.example.test`
                      }
                    />
                  </label>
                </div>

                <div className="rounded border border-border bg-background/40 p-3">
                  <label className="flex items-center gap-3">
                    <Checkbox
                      checked={draft.enabled}
                      onCheckedChange={(checked) =>
                        setDraft((previous) => ({
                          ...previous,
                          enabled: checked === true,
                        }))
                      }
                    />
                    <span className="text-sm font-medium">{t("settings.mediaServerEnabled")}</span>
                  </label>
                </div>

                {draft.provider === "plex" ? (
                  <label className="block">
                    <Label className="mb-2 block" htmlFor="settings-media-server-machine-id">
                      {t("settings.plexMachineId")}
                    </Label>
                    <Input
                      id="settings-media-server-machine-id"
                      value={draft.machineId}
                      onChange={(event) =>
                        setDraft((previous) => ({
                          ...previous,
                          machineId: event.target.value,
                        }))
                      }
                      placeholder={t("settings.plexMachineIdPlaceholder")}
                    />
                  </label>
                ) : null}

                {draft.provider === "jellyfin" || draft.provider === "emby" ? (
                  <div className="grid gap-3 md:grid-cols-2">
                    <label>
                      <Label className="mb-2 block" htmlFor="settings-media-server-api-key">
                        {t("settings.apiKey")}
                      </Label>
                      <Input
                        id="settings-media-server-api-key"
                        value={draft.apiKey}
                        onChange={(event) =>
                          setDraft((previous) => ({
                            ...previous,
                            apiKey: event.target.value,
                            clearApiKey: false,
                          }))
                        }
                        type="password"
                        placeholder={t("form.apiKeyInputPlaceholder")}
                      />
                    </label>
                    {editingConnectionId ? (
                      <label className="flex items-end gap-2 pb-2">
                        <Checkbox
                          checked={draft.clearApiKey}
                          onCheckedChange={(checked) =>
                            setDraft((previous) => ({
                              ...previous,
                              clearApiKey: checked === true,
                            }))
                          }
                        />
                        <span className="text-sm">{t("settings.clearSavedApiKey")}</span>
                      </label>
                    ) : null}
                    {draft.provider === "jellyfin" ? (
                      <>
                        <label>
                          <Label className="mb-2 block" htmlFor="settings-media-server-admin-username">
                            {t("settings.adminUsername")}
                          </Label>
                          <Input
                            id="settings-media-server-admin-username"
                            value={draft.adminUsername}
                            onChange={(event) =>
                              setDraft((previous) => ({
                                ...previous,
                                adminUsername: event.target.value,
                              }))
                            }
                            autoComplete="off"
                            placeholder={t("form.usernamePlaceholder")}
                          />
                        </label>
                        <label>
                          <Label className="mb-2 block" htmlFor="settings-media-server-admin-password">
                            {t("settings.adminPassword")}
                          </Label>
                          <Input
                            id="settings-media-server-admin-password"
                            value={draft.adminPassword}
                            onChange={(event) =>
                              setDraft((previous) => ({
                                ...previous,
                                adminPassword: event.target.value,
                              }))
                            }
                            type="password"
                            autoComplete="off"
                            placeholder={t("form.passwordPlaceholder")}
                          />
                        </label>
                      </>
                    ) : null}
                    <div className="md:col-span-2">
                      <LocalRemotePathMappingsField
                        fieldKey="path_mappings"
                        label={t("settings.mediaServerPathMappings")}
                        value={draft.pathMappingsText}
                        helpText={t("settings.mediaServerPathMappingsHelp")}
                        direction="remote-to-local"
                        onChange={(_, value) =>
                          setDraft((previous) => ({
                            ...previous,
                            pathMappingsText: value,
                          }))
                        }
                      />
                    </div>
                  </div>
                ) : null}

                {supportsAuth ? (
                  <div className="space-y-3 rounded border border-border bg-background/40 p-3">
                    <div className="font-medium">{t("settings.mediaServerAuthCapabilities")}</div>
                    <div className="grid gap-3 md:grid-cols-3">
                      <label className="flex items-center gap-2">
                        <Checkbox
                          checked={draft.loginEnabled}
                          onCheckedChange={(checked) =>
                            setDraft((previous) => ({
                              ...previous,
                              loginEnabled: checked === true,
                            }))
                          }
                        />
                        <span>{t("settings.authProviderLoginEnabled")}</span>
                      </label>
                      <label className="flex items-center gap-2">
                        <Checkbox
                          checked={draft.linkingEnabled}
                          onCheckedChange={(checked) =>
                            setDraft((previous) => ({
                              ...previous,
                              linkingEnabled: checked === true,
                            }))
                          }
                        />
                        <span>{t("settings.authProviderLinkingEnabled")}</span>
                      </label>
                      <label className="flex items-center gap-2">
                        <Checkbox
                          checked={draft.autoAddEnabled}
                          onCheckedChange={(checked) =>
                            setDraft((previous) => ({
                              ...previous,
                              autoAddEnabled: checked === true,
                            }))
                          }
                        />
                        <span>{t("settings.mediaServerAutoAddEnabled")}</span>
                      </label>
                    </div>

                    <div className="grid gap-3 lg:grid-cols-[minmax(0,22rem)_minmax(0,1fr)]">
                      <div className="rounded border border-border bg-card/50 p-3">
                        <Label className="mb-2 block">{t("settings.defaultAppPermissions")}</Label>
                        <div className="grid gap-2">
                          {APP_PERMISSION_OPTIONS.map((permission) => (
                            <label key={permission.value} className="flex items-center gap-2">
                              <Checkbox
                                checked={draft.defaultAppPermissions.includes(permission.value)}
                                onCheckedChange={() =>
                                  setDraft((previous) => ({
                                    ...previous,
                                    defaultAppPermissions: toggleListValue(
                                      previous.defaultAppPermissions as AppPermission[],
                                      permission.value,
                                    ),
                                  }))
                                }
                              />
                              <span>{permission.label}</span>
                            </label>
                          ))}
                        </div>
                      </div>
                      <div className="rounded border border-border bg-card/50 p-3">
                        <Label className="mb-2 block">{t("settings.defaultLibraryPermissions")}</Label>
                        <div className="space-y-3">
                          {libraries.map((library) => {
                            const selected = selectedLibraryGrant(
                              draft.defaultLibraryGrants,
                              library.id,
                            );
                            return (
                              <div key={library.id} className="space-y-2">
                                <div className="font-medium">{library.name}</div>
                                <div className="flex flex-wrap gap-2">
                                  {LIBRARY_PERMISSION_OPTIONS.map((permission) => (
                                    <label
                                      key={`${library.id}-${permission.value}`}
                                      className="flex min-h-8 items-center gap-2 rounded-md px-2 py-1 hover:bg-card/70"
                                    >
                                      <Checkbox
                                        checked={selected.includes(permission.value)}
                                        onCheckedChange={() => {
                                          const next = toggleListValue(
                                            selected as LibraryPermission[],
                                            permission.value,
                                          );
                                          setDraft((previous) => ({
                                            ...previous,
                                            defaultLibraryGrants: updateLibraryGrant(
                                              previous.defaultLibraryGrants,
                                              library.id,
                                              next,
                                            ),
                                          }));
                                        }}
                                      />
                                      <span className="text-xs">{permission.label}</span>
                                    </label>
                                  ))}
                                </div>
                              </div>
                            );
                          })}
                        </div>
                      </div>
                    </div>
                  </div>
                ) : null}

                <div className="flex gap-2">
                  <Button
                    id="settings-media-server-save"
                    type="submit"
                    disabled={mutatingConnectionId === "new" || mutatingConnectionId === editingConnectionId}
                  >
                    {mutatingConnectionId === "new" || mutatingConnectionId === editingConnectionId
                      ? t("label.saving")
                      : isEditing
                        ? t("settings.mediaServerUpdate")
                        : t("settings.mediaServerCreate")}
                  </Button>
                  <Button
                    id="settings-media-server-cancel"
                    type="button"
                    variant="outline"
                    onClick={resetDraft}
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
                id="settings-media-server-create"
                type="button"
                size="lg"
                onClick={startCreateConnection}
                disabled={mutatingConnectionId !== null}
                className="h-12 border border-emerald-500/30 bg-emerald-500/15 px-5 text-base font-semibold text-emerald-100 hover:bg-emerald-500/25 hover:text-emerald-50"
              >
                <Plus className="h-5 w-5" />
                {t("settings.mediaServerCreateNew")}
              </Button>
            </div>
          ) : null}
        </>
      ) : (
        <div className="flex justify-center">
          <Button
            id="settings-media-server-create"
            type="button"
            size="lg"
            onClick={startCreateConnection}
            className="h-12 border border-emerald-500/30 bg-emerald-500/15 px-5 text-base font-semibold text-emerald-100 hover:bg-emerald-500/25 hover:text-emerald-50"
          >
            <Plus className="h-5 w-5" />
            {t("settings.mediaServerCreateNew")}
          </Button>
        </div>
      )}
    </div>
  );
}
