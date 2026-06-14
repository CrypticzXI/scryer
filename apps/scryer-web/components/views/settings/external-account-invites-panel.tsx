import * as React from "react";
import { Check, ChevronsUpDown, Loader2, Plus, Search } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
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
import { isVisibleExternalAccountProvider } from "@/lib/constants/integration-providers";
import { useTranslate } from "@/lib/context/translate-context";
import type {
  ExternalAccountProvider,
  ExternalAuthRuntimeConnection,
  ExternalAuthRuntimeSettings,
  LinkedAccount,
} from "@/lib/types/settings";
import { selectorId } from "@/lib/utils/dom-ids";

export type ExternalInviteDraft = {
  userId: string;
  provider: ExternalAccountProvider;
  connectionId: string;
  providerUserIdentifier: string;
  providerUserId: string;
};

export type ExternalInviteUser = {
  id: string;
  username: string;
};

export type ExternalInviteProviderUserOption = {
  id: string;
  username: string;
  displayName: string | null;
  avatarUrl: string | null;
};

type ExternalAccountInvitesPanelProps = {
  users: ExternalInviteUser[];
  invites: LinkedAccount[];
  providerUserOptions: ExternalInviteProviderUserOption[];
  providerUserSearchLoading: boolean;
  providerUserLookupError: string | null;
  externalAuthSettings: ExternalAuthRuntimeSettings;
  loading: boolean;
  externalInviteDraft: ExternalInviteDraft;
  externalInviteSubmitting: boolean;
  updateExternalInviteDraft: (patch: Partial<ExternalInviteDraft>) => void;
  createExternalAccountInvite: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
};

function providerLabel(provider: ExternalAccountProvider): string {
  switch (provider) {
    case "plex":
      return "Plex";
    case "jellyfin":
      return "Jellyfin";
    default:
      return provider;
  }
}

function providerConnections(
  settings: ExternalAuthRuntimeSettings,
  provider: ExternalAccountProvider,
): ExternalAuthRuntimeConnection[] {
  return settings.connections.filter(
    (connection) => connection.provider === provider && connection.loginEnabled,
  );
}

function providerConnectionLabel(connection: ExternalAuthRuntimeConnection): string {
  return connection.displayName;
}

function inviteConnectionLabel(
  settings: ExternalAuthRuntimeSettings,
  invite: LinkedAccount,
): string {
  const connection = providerConnections(settings, invite.provider).find(
    (candidate) => candidate.id === invite.connectionId,
  );
  if (connection) {
    return providerConnectionLabel(connection);
  }

  return invite.provider === "jellyfin"
    ? providerLabel(invite.provider)
    : invite.connectionId;
}

function providerIdentifierLabel(provider: ExternalAccountProvider, t: ReturnType<typeof useTranslate>): string {
  return provider === "jellyfin"
    ? t("settings.jellyfinUsername")
    : t("settings.plexUserId");
}

function inviteProviderOptions(settings: ExternalAuthRuntimeSettings): ExternalAccountProvider[] {
  const providers: ExternalAccountProvider[] = [];
  for (const connection of settings.connections) {
    if (
      !connection.loginEnabled ||
      !settings.loginProviders.includes(connection.provider) ||
      !isVisibleExternalAccountProvider(connection.provider) ||
      providers.includes(connection.provider)
    ) {
      continue;
    }
    providers.push(connection.provider);
  }
  return providers;
}

function formatTimestamp(value: string | null | undefined): string {
  if (!value) {
    return "-";
  }

  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) {
    return value;
  }

  return parsed.toLocaleString();
}

function providerIdentityLabel(account: LinkedAccount): string {
  const name = account.displayName || account.username;
  return account.externalUserId ? `${name} (${account.externalUserId})` : name;
}

function ProviderAvatar({
  avatarUrl,
  label,
}: {
  avatarUrl: string | null | undefined;
  label: string;
}) {
  return avatarUrl ? (
    <img
      src={avatarUrl}
      alt=""
      className="h-7 w-7 shrink-0 rounded-full border border-border object-cover"
      loading="lazy"
    />
  ) : (
    <span
      aria-hidden="true"
      className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full border border-border bg-muted text-xs font-medium text-muted-foreground"
    >
      {label.trim().slice(0, 1).toUpperCase() || "?"}
    </span>
  );
}

function JellyfinProviderUserCombobox({
  id,
  value,
  options,
  selectedOption,
  loading,
  disabled,
  placeholder,
  emptyLabel,
  loadingLabel,
  onChange,
  onSelectOption,
}: {
  id: string;
  value: string;
  options: ExternalInviteProviderUserOption[];
  selectedOption: ExternalInviteProviderUserOption | null;
  loading: boolean;
  disabled: boolean;
  placeholder: string;
  emptyLabel: string;
  loadingLabel: string;
  onChange: (value: string) => void;
  onSelectOption: (option: ExternalInviteProviderUserOption) => void;
}) {
  const [open, setOpen] = React.useState(false);
  const normalizedValue = value.trim().toLocaleLowerCase();
  const filteredOptions = React.useMemo(() => {
    if (!normalizedValue) {
      return options;
    }

    return options.filter((option) => {
      const searchable = [
        option.username,
        option.id,
        option.displayName ?? "",
      ]
        .join(" ")
        .toLocaleLowerCase();
      return searchable.includes(normalizedValue);
    });
  }, [normalizedValue, options]);

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          id={id}
          type="button"
          variant="outline"
          role="combobox"
          aria-expanded={open}
          className="h-9 w-full justify-between border-input bg-field px-3 text-left font-normal shadow-xs hover:bg-field hover:text-foreground"
          disabled={disabled}
        >
          <span className="flex min-w-0 items-center gap-2">
            {selectedOption ? (
              <ProviderAvatar
                avatarUrl={selectedOption.avatarUrl}
                label={selectedOption.displayName ?? selectedOption.username}
              />
            ) : null}
            <span
              className={
                value.trim()
                  ? "truncate"
                  : "truncate text-muted-foreground"
              }
            >
              {selectedOption
                ? selectedOption.displayName ?? selectedOption.username
                : value.trim() || placeholder}
            </span>
          </span>
          {loading ? (
            <Loader2 className="h-4 w-4 shrink-0 animate-spin text-muted-foreground" />
          ) : (
            <ChevronsUpDown className="h-4 w-4 shrink-0 text-muted-foreground" />
          )}
        </Button>
      </PopoverTrigger>
      <PopoverContent
        align="start"
        className="w-[var(--radix-popover-trigger-width)] min-w-64 p-0"
        onOpenAutoFocus={(event) => event.preventDefault()}
      >
        <Command shouldFilter={false}>
          <div className="border-b border-border p-2">
            <div className="flex h-8 items-center gap-2 rounded-md border border-input bg-field px-2">
              <Search className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
              <input
                id={`${id}-search`}
                type="text"
                value={value}
                onChange={(event) => onChange(event.target.value)}
                placeholder={placeholder}
                autoComplete="off"
                data-1p-ignore="true"
                data-lpignore="true"
                data-form-type="other"
                name="jellyfin-provider-user-search"
                className="w-full bg-transparent text-sm text-foreground placeholder:text-muted-foreground outline-none"
              />
            </div>
          </div>
          <CommandList>
            {loading ? (
              <div className="flex items-center gap-2 px-3 py-3 text-sm text-muted-foreground">
                <Loader2 className="h-4 w-4 animate-spin" />
                <span>{loadingLabel}</span>
              </div>
            ) : null}
            {!loading && filteredOptions.length === 0 ? (
              <CommandEmpty>{emptyLabel}</CommandEmpty>
            ) : null}
            <CommandGroup>
              {filteredOptions.map((option) => {
                const label = option.displayName ?? option.username;
                const selected = selectedOption?.id === option.id;
                return (
                  <CommandItem
                    id={selectorId("settings-external-invite-provider-user-option", option.username)}
                    key={option.id}
                    value={`${option.username} ${option.displayName ?? ""} ${option.id}`}
                    onSelect={() => {
                      onSelectOption(option);
                      setOpen(false);
                    }}
                    className="items-center gap-3"
                  >
                    <ProviderAvatar avatarUrl={option.avatarUrl} label={label} />
                    <span className="min-w-0 flex-1">
                      <span className="block truncate font-medium">{label}</span>
                      <span className="block truncate text-xs text-muted-foreground">
                        {option.username}
                      </span>
                    </span>
                    {selected ? <Check className="h-4 w-4 text-primary" /> : null}
                  </CommandItem>
                );
              })}
            </CommandGroup>
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  );
}

function inviteStatus(account: LinkedAccount, t: ReturnType<typeof useTranslate>) {
  if (account.status === "disabled") {
    return {
      label: t("settings.externalAccountInviteStatusDisabled"),
      className: "border-destructive/40 bg-destructive/10 text-destructive",
    };
  }

  if (account.lastLoginAt) {
    return {
      label: t("settings.externalAccountInviteStatusLoggedIn"),
      className: "border-emerald-500/40 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
    };
  }

  if (account.status === "pending_claim") {
    return {
      label: t("settings.externalAccountInviteStatusPending"),
      className: "border-amber-500/40 bg-amber-500/10 text-amber-700 dark:text-amber-300",
    };
  }

  return {
    label: t("settings.externalAccountInviteStatusActive"),
    className: "border-border bg-background text-muted-foreground",
  };
}

export function ExternalAccountInvitesPanel({
  users,
  invites,
  providerUserOptions,
  providerUserSearchLoading,
  providerUserLookupError,
  externalAuthSettings,
  loading,
  externalInviteDraft,
  externalInviteSubmitting,
  updateExternalInviteDraft,
  createExternalAccountInvite,
}: ExternalAccountInvitesPanelProps) {
  const t = useTranslate();
  const inviteProviders = inviteProviderOptions(externalAuthSettings);
  const inviteConnections = providerConnections(externalAuthSettings, externalInviteDraft.provider);
  const providerIdentifierLabelText = providerIdentifierLabel(externalInviteDraft.provider, t);
  const isJellyfinInvite = externalInviteDraft.provider === "jellyfin";
  const inviteUnavailable = inviteProviders.length === 0 || users.length === 0;
  const inviteCreateDisabled =
    externalInviteSubmitting ||
    !externalInviteDraft.userId ||
    !externalInviteDraft.connectionId ||
    (isJellyfinInvite
      ? externalInviteDraft.providerUserId.trim().length === 0
      : externalInviteDraft.providerUserIdentifier.trim().length === 0);
  const usersById = new Map(users.map((user) => [user.id, user.username]));
  const sortedInvites = invites
    .filter((invite) => isVisibleExternalAccountProvider(invite.provider))
    .sort((left, right) => {
      const rightTime = new Date(right.createdAt).getTime();
      const leftTime = new Date(left.createdAt).getTime();
      return (Number.isNaN(rightTime) ? 0 : rightTime) - (Number.isNaN(leftTime) ? 0 : leftTime);
    });
  const selectedProviderUser = isJellyfinInvite
    ? providerUserOptions.find((option) => option.id === externalInviteDraft.providerUserId) ?? null
    : null;

  return (
    <div className="space-y-4 rounded-lg border border-border bg-card/50 p-4">
      <div className="flex items-center gap-2">
        <h3 className="text-base font-medium">{t("settings.externalAccountInvites")}</h3>
        {loading ? <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" /> : null}
      </div>

      {inviteUnavailable ? (
        <p className="text-muted-foreground">
          {t("settings.externalAccountInvitesUnavailable")}
        </p>
      ) : (
        <form
          id="settings-external-account-invite-form"
          className="space-y-4"
          onSubmit={createExternalAccountInvite}
        >
          <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-5">
            <div className="space-y-1.5">
              <Label htmlFor="settings-external-invite-user">
                {t("settings.user")}
              </Label>
              <Select
                value={externalInviteDraft.userId}
                onValueChange={(userId) => updateExternalInviteDraft({ userId })}
                disabled={externalInviteSubmitting}
              >
                <SelectTrigger id="settings-external-invite-user" className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                {users.map((user) => (
                  <SelectItem
                    id={selectorId("settings-external-invite-user-option", user.username)}
                    key={user.id}
                    value={user.id}
                  >
                    {user.username}
                  </SelectItem>
                ))}
                </SelectContent>
              </Select>
            </div>
            {inviteProviders.length > 1 ? (
              <div className="space-y-1.5">
                <Label htmlFor="settings-external-invite-provider">
                  {t("settings.provider")}
                </Label>
                <Select
                  value={externalInviteDraft.provider}
                  onValueChange={(value) => {
                    const provider = value as ExternalAccountProvider;
                    updateExternalInviteDraft({
                      provider,
                      connectionId: providerConnections(externalAuthSettings, provider)[0]?.id ?? "",
                      providerUserIdentifier: "",
                      providerUserId: "",
                    });
                  }}
                  disabled={externalInviteSubmitting}
                >
                  <SelectTrigger id="settings-external-invite-provider" className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                  {inviteProviders.map((provider) => (
                    <SelectItem key={provider} value={provider}>
                      {providerLabel(provider)}
                    </SelectItem>
                  ))}
                  </SelectContent>
                </Select>
              </div>
            ) : null}
            {inviteConnections.length > 1 ? (
              <div className="space-y-1.5">
                <Label htmlFor="settings-external-invite-connection">
                  {t("profile.linkedAccountConnection")}
                </Label>
                <Select
                  value={externalInviteDraft.connectionId}
                  onValueChange={(connectionId) =>
                    updateExternalInviteDraft({
                      connectionId,
                      providerUserIdentifier: "",
                      providerUserId: "",
                    })
                  }
                  disabled={externalInviteSubmitting}
                >
                  <SelectTrigger id="settings-external-invite-connection" className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                  {inviteConnections.map((connection) => (
                    <SelectItem key={connection.id} value={connection.id}>
                      {providerConnectionLabel(connection)}
                    </SelectItem>
                  ))}
                  </SelectContent>
                </Select>
              </div>
            ) : null}
            <div className="space-y-1.5">
              <div className="flex items-center justify-between gap-2">
                <Label htmlFor="settings-external-invite-provider-identifier">
                  {providerIdentifierLabelText}
                </Label>
                {isJellyfinInvite && providerUserSearchLoading ? (
                  <span className="inline-flex items-center gap-1 text-xs text-muted-foreground">
                    <Loader2 className="h-3 w-3 animate-spin" />
                    {t("label.loading")}
                  </span>
                ) : null}
              </div>
              {isJellyfinInvite ? (
                <JellyfinProviderUserCombobox
                  id="settings-external-invite-provider-identifier"
                  value={externalInviteDraft.providerUserIdentifier}
                  options={providerUserOptions}
                  selectedOption={selectedProviderUser}
                  loading={providerUserSearchLoading}
                  disabled={externalInviteSubmitting}
                  placeholder={providerIdentifierLabelText}
                  emptyLabel={t("settings.jellyfinUserPickerEmpty")}
                  loadingLabel={t("label.loading")}
                  onChange={(providerUserIdentifier) =>
                    updateExternalInviteDraft({ providerUserIdentifier, providerUserId: "" })
                  }
                  onSelectOption={(option) =>
                    updateExternalInviteDraft({
                      providerUserIdentifier: option.username,
                      providerUserId: option.id,
                    })
                  }
                />
              ) : (
                <Input
                  id="settings-external-invite-provider-identifier"
                  type="text"
                  value={externalInviteDraft.providerUserIdentifier}
                  onChange={(event) =>
                    updateExternalInviteDraft({ providerUserIdentifier: event.target.value })
                  }
                  disabled={externalInviteSubmitting}
                  placeholder={providerIdentifierLabelText}
                  autoComplete="off"
                  data-1p-ignore="true"
                  data-lpignore="true"
                  data-form-type="other"
                  name="external-provider-user-identifier"
                  required
                />
              )}
              {isJellyfinInvite && providerUserLookupError ? (
                <p className="text-xs text-destructive">{providerUserLookupError}</p>
              ) : null}
            </div>
            <div className="space-y-1.5">
              <div aria-hidden="true" className="h-3.5" />
              <Button
                id="settings-external-account-invite-create"
                type="submit"
                className="min-w-40"
                disabled={inviteCreateDisabled}
              >
                {externalInviteSubmitting ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <Plus className="h-4 w-4" />
                )}
                {externalInviteSubmitting ? t("label.saving") : t("settings.createInvite")}
              </Button>
            </div>
          </div>
        </form>
      )}

      <div className="space-y-3">
        <h4 className="text-sm font-medium">{t("settings.previousExternalAccountInvites")}</h4>
        <div className="rounded border border-border">
          <div className="overflow-x-auto">
            <Table id="settings-external-account-invites-table">
              <TableHeader>
                <TableRow>
                  <TableHead>{t("settings.user")}</TableHead>
                  <TableHead>{t("settings.provider")}</TableHead>
                  <TableHead>{t("profile.linkedAccountConnection")}</TableHead>
                  <TableHead>{t("settings.externalAccountInviteProviderUser")}</TableHead>
                  <TableHead>{t("profile.linkedAccountStatus")}</TableHead>
                  <TableHead>{t("settings.externalAccountInviteCreatedAt")}</TableHead>
                  <TableHead>{t("settings.externalAccountInviteLastLogin")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {loading ? (
                  <TableRow>
                    <TableCell colSpan={7} className="text-muted-foreground">
                      {t("label.loading")}
                    </TableCell>
                  </TableRow>
                ) : sortedInvites.length === 0 ? (
                  <TableRow>
                    <TableCell colSpan={7} className="text-muted-foreground">
                      {t("settings.noExternalAccountInvites")}
                    </TableCell>
                  </TableRow>
                ) : (
                  sortedInvites.map((invite) => {
                    const status = inviteStatus(invite, t);
                    return (
                      <TableRow
                        key={invite.id}
                        id={selectorId(
                          "settings-external-account-invite-row",
                          usersById.get(invite.userId) ?? invite.userId,
                          invite.provider,
                          invite.username,
                        )}
                      >
                        <TableCell>{usersById.get(invite.userId) ?? invite.userId}</TableCell>
                        <TableCell>{providerLabel(invite.provider)}</TableCell>
                        <TableCell>{inviteConnectionLabel(externalAuthSettings, invite)}</TableCell>
                        <TableCell>
                          <div className="flex items-center gap-2">
                            <ProviderAvatar
                              avatarUrl={invite.avatarUrl}
                              label={invite.displayName ?? invite.username}
                            />
                            <span>{providerIdentityLabel(invite)}</span>
                          </div>
                        </TableCell>
                        <TableCell>
                          <span
                            className={`inline-flex rounded-full border px-2 py-0.5 text-xs font-medium ${status.className}`}
                          >
                            {status.label}
                          </span>
                        </TableCell>
                        <TableCell>{formatTimestamp(invite.createdAt)}</TableCell>
                        <TableCell>{formatTimestamp(invite.lastLoginAt)}</TableCell>
                      </TableRow>
                    );
                  })
                )}
              </TableBody>
            </Table>
          </div>
        </div>
      </div>
    </div>
  );
}
