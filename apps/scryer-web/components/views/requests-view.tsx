import * as React from "react";
import {
  Check,
  CircleCheck,
  Clock,
  Gem,
  History,
  Inbox,
  Loader2,
  Pencil,
  RefreshCw,
  ShieldX,
  User,
  X,
  type LucideIcon,
} from "lucide-react";

import { LibraryMultiSelect } from "@/components/common/library-multi-select";
import { TitlePoster } from "@/components/title-poster";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import type { LibraryRecord, MediaRequestRecord } from "@/lib/types";
import { formatUiDateTime } from "@/lib/utils/date-format";
import {
  mediaRequestApproveId,
  mediaRequestCancelId,
  mediaRequestDismissId,
  mediaRequestEditId,
  mediaRequestMonitorOptionId,
  mediaRequestProfileOptionId,
  mediaRequestRowId,
  mediaRequestStatusId,
} from "@/lib/utils/dom-ids";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";
import { cn } from "@/lib/utils";

type QualityProfileOption = {
  id: string;
  name: string;
};

type RequestsMode = "admin" | "mine";
type RequestStatusFilter = "all" | MediaRequestRecord["status"];
type RequestFacetFilter = MediaRequestRecord["facet"];

type RequestMonitorType =
  | "monitored"
  | "unmonitored"
  | "futureEpisodes"
  | "missingAndFutureEpisodes"
  | "allEpisodes"
  | "none";

type UpdateRequestValues = {
  requestedQualityProfileId: string;
  requestedMonitorType?: RequestMonitorType;
};

type ApproveRequestValues = {
  qualityProfileId: string;
  monitorType?: RequestMonitorType;
};

type RequestsViewProps = {
  mode: RequestsMode;
  canShowAdminMode: boolean;
  canShowRequesterMode: boolean;
  onModeChange: (mode: RequestsMode) => void;
  statusFilter: RequestStatusFilter;
  onStatusFilterChange: (status: RequestStatusFilter) => void;
  libraries: LibraryRecord[];
  selectedLibraryIds: string[];
  onSelectedLibraryIdsChange: (libraryIds: string[]) => void;
  requests: MediaRequestRecord[];
  qualityProfileOptions: QualityProfileOption[];
  loading: boolean;
  actionRequestId: string | null;
  onRefresh: () => void;
  onLoadQualityProfileOptions: () => void;
  onApprove: (request: MediaRequestRecord, values: ApproveRequestValues) => void;
  onDismiss: (request: MediaRequestRecord) => void;
  onUpdateRequest: (request: MediaRequestRecord, values: UpdateRequestValues) => void;
  onCancelRequest: (request: MediaRequestRecord) => void;
};

function requestExternalIdLabel(request: MediaRequestRecord): string {
  return request.externalIds
    .map((externalId) => `${externalId.source.toUpperCase()} ${externalId.value}`)
    .join(" / ");
}

function requesterLabel(request: MediaRequestRecord): string {
  return request.requesters
    .map((requester) => requester.username)
    .filter(Boolean)
    .join(", ");
}

function requestExternalIdValue(
  request: MediaRequestRecord,
  source: string,
): string | undefined {
  return request.externalIds.find(
    (externalId) => externalId.source.toLowerCase() === source,
  )?.value;
}

function RequesterAvatarStack({ request }: { request: MediaRequestRecord }) {
  const avatarRequesters = request.requesters.filter((requester) =>
    requester.avatarUrl?.trim(),
  );
  if (avatarRequesters.length === 0) {
    return null;
  }
  return (
    <span className="inline-flex -space-x-2 align-middle">
      {avatarRequesters.map((requester) => (
        <img
          key={requester.userId}
          src={requester.avatarUrl ?? ""}
          alt=""
          title={requester.username}
          className="h-6 w-6 rounded-full border border-background bg-muted object-cover ring-1 ring-border"
          loading="lazy"
        />
      ))}
    </span>
  );
}

function profileLabel(
  profileId: string | null | undefined,
  profileName: string | null | undefined,
  qualityProfileOptions: QualityProfileOption[],
): string | null {
  if (profileName?.trim()) {
    return profileName.trim();
  }
  const normalizedId = profileId?.trim();
  if (!normalizedId) {
    return null;
  }
  return (
    qualityProfileOptions.find((profile) => profile.id === normalizedId)?.name ??
    normalizedId
  );
}

function monitorTypeLabel(t: ReturnType<typeof useTranslate>, value: string | null | undefined): string | null {
  switch (value) {
    case "monitored":
      return t("search.monitorType.monitored");
    case "unmonitored":
      return t("search.monitorType.unmonitored");
    case "futureepisodes":
    case "futureEpisodes":
      return t("search.monitorType.futureEpisodes");
    case "missingandfutureepisodes":
    case "missingAndFutureEpisodes":
      return t("search.monitorType.missingAndFutureEpisodes");
    case "allepisodes":
    case "allEpisodes":
      return t("search.monitorType.allEpisodes");
    case "none":
      return t("search.monitorType.none");
    default:
      return value?.trim() || null;
  }
}

function monitorTypeSelectValue(
  facet: MediaRequestRecord["facet"],
  value: string | null | undefined,
): RequestMonitorType {
  const normalized = value?.replace(/[-_\s]/g, "").toLowerCase();
  switch (normalized) {
    case "monitored":
      return "monitored";
    case "unmonitored":
      return "unmonitored";
    case "missingandfutureepisodes":
      return "missingAndFutureEpisodes";
    case "allepisodes":
      return "allEpisodes";
    case "none":
      return "none";
    case "futureepisodes":
    default:
      return facet === "movie" ? "monitored" : "futureEpisodes";
  }
}

function monitorOptions(t: ReturnType<typeof useTranslate>): Array<{ value: RequestMonitorType; label: string }> {
  return [
    { value: "futureEpisodes", label: t("search.monitorType.futureEpisodes") },
    {
      value: "missingAndFutureEpisodes",
      label: t("search.monitorType.missingAndFutureEpisodes"),
    },
    { value: "allEpisodes", label: t("search.monitorType.allEpisodes") },
    { value: "none", label: t("search.monitorType.none") },
  ];
}

function requestProfileOptionsForLibrary(
  libraries: LibraryRecord[],
  libraryId: string,
  qualityProfileOptions: QualityProfileOption[],
): QualityProfileOption[] {
  const library = libraries.find((library) => library.id === libraryId);
  const requestProfileIds = library?.requestQualityProfileIds?.length
    ? library.requestQualityProfileIds
    : library?.requestQualityProfileDefaultId
      ? [library.requestQualityProfileDefaultId]
      : [];
  return requestProfileIds.map((profileId) => {
    const profile = qualityProfileOptions.find((option) => option.id === profileId);
    return {
      id: profileId,
      name: profile?.name ?? profileId,
    };
  });
}

function requestStatusLabel(t: ReturnType<typeof useTranslate>, status: MediaRequestRecord["status"]): string {
  switch (status) {
    case "pending":
      return t("requests.status.pending");
    case "approved":
      return t("requests.status.approved");
    case "rejected":
      return "Dismissed";
    case "canceled":
      return t("requests.status.canceled");
    default:
      return status;
  }
}

function requestStatusTone(
  t: ReturnType<typeof useTranslate>,
  status: MediaRequestRecord["status"],
): { label: string; Icon: LucideIcon; className: string } {
  switch (status) {
    case "pending":
      return {
        label: requestStatusLabel(t, status),
        Icon: Clock,
        className: "border-amber-400/30 bg-amber-500/15 text-amber-300",
      };
    case "approved":
      return {
        label: requestStatusLabel(t, status),
        Icon: Check,
        className: "border-emerald-400/25 bg-emerald-500/15 text-emerald-300",
      };
    case "rejected":
      return {
        label: requestStatusLabel(t, status),
        Icon: X,
        className: "border-red-400/25 bg-red-500/15 text-red-300",
      };
    case "canceled":
    default:
      return {
        label: requestStatusLabel(t, status),
        Icon: ShieldX,
        className: "border-border bg-background text-muted-foreground",
      };
  }
}

function statusFilterOptions(mode: RequestsMode): Array<{
  value: RequestStatusFilter;
  label: string;
}> {
  if (mode === "admin") {
    return [
      { value: "pending", label: "Pending" },
      { value: "approved", label: "Approved" },
      { value: "rejected", label: "Dismissed" },
    ];
  }

  return [
    { value: "all", label: "All" },
    { value: "pending", label: "Pending" },
    { value: "approved", label: "Approved" },
    { value: "rejected", label: "Dismissed" },
    { value: "canceled", label: "Canceled" },
  ];
}

function requestCountByFacet(
  requests: MediaRequestRecord[],
  facet: RequestFacetFilter,
): number {
  return requests.filter((request) => request.facet === facet).length;
}

export function RequestsView({
  mode,
  canShowAdminMode,
  canShowRequesterMode,
  onModeChange,
  statusFilter,
  onStatusFilterChange,
  libraries,
  selectedLibraryIds,
  onSelectedLibraryIdsChange,
  requests,
  qualityProfileOptions,
  loading,
  actionRequestId,
  onRefresh,
  onLoadQualityProfileOptions,
  onApprove,
  onDismiss,
  onUpdateRequest,
  onCancelRequest,
}: RequestsViewProps) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const showModeSwitch = canShowAdminMode && canShowRequesterMode;
  const HeadingIcon = mode === "admin" ? Inbox : Clock;
  const headingTitle = mode === "admin" ? "Request Queue" : "My Requests";
  const headingCopy =
    mode === "admin"
      ? "Approve or dismiss member requests. Approved titles are added to the library and start searching."
      : "Track the titles you've asked Scryer to grab. You'll be notified when they're available.";
  const filters = statusFilterOptions(mode);
  const [adminFacetFilters, setAdminFacetFilters] = React.useState<
    Record<RequestFacetFilter, boolean>
  >({ movie: true, series: true, anime: true });
  const displayedRequests = React.useMemo(
    () =>
      mode === "admin"
        ? requests.filter((request) => adminFacetFilters[request.facet])
        : requests,
    [adminFacetFilters, mode, requests],
  );
  const [approvalRequest, setApprovalRequest] =
    React.useState<MediaRequestRecord | null>(null);
  const [approvalProfileId, setApprovalProfileId] = React.useState("");
  const [approvalMonitorType, setApprovalMonitorType] =
    React.useState<RequestMonitorType>("futureEpisodes");
  const [editRequest, setEditRequest] =
    React.useState<MediaRequestRecord | null>(null);
  const [editProfileId, setEditProfileId] = React.useState("");
  const [editMonitorType, setEditMonitorType] =
    React.useState<RequestMonitorType>("futureEpisodes");
  const editProfileOptions = React.useMemo(
    () =>
      editRequest
        ? requestProfileOptionsForLibrary(
            libraries,
            editRequest.libraryId,
            qualityProfileOptions,
          )
        : [],
    [editRequest, libraries, qualityProfileOptions],
  );

  React.useEffect(() => {
    if (!approvalRequest) return;
    const requestedProfileId = approvalRequest.requestedQualityProfileId?.trim() ?? "";
    const requestedStillValid = qualityProfileOptions.some(
      (profile) => profile.id === requestedProfileId,
    );
    const library = libraries.find(
      (library) => library.id === approvalRequest.libraryId,
    );
    const libraryDefaultProfileId =
      library?.qualityProfileId?.trim() ??
      library?.requestQualityProfileDefaultId?.trim() ??
      "";
    const libraryDefaultStillValid = qualityProfileOptions.some(
      (profile) => profile.id === libraryDefaultProfileId,
    );
    setApprovalProfileId(
      requestedStillValid
        ? requestedProfileId
        : libraryDefaultStillValid
          ? libraryDefaultProfileId
          : qualityProfileOptions[0]?.id ?? "",
    );
    setApprovalMonitorType(
      monitorTypeSelectValue(
        approvalRequest.facet,
        approvalRequest.requestedMonitorType,
      ),
    );
  }, [approvalRequest, libraries, qualityProfileOptions]);

  React.useEffect(() => {
    if (!editRequest) return;
    const requestedProfileId = editRequest.requestedQualityProfileId?.trim() ?? "";
    const requestedStillAllowed = editProfileOptions.some(
      (profile) => profile.id === requestedProfileId,
    );
    setEditProfileId(
      requestedStillAllowed
        ? requestedProfileId
        : editProfileOptions[0]?.id ?? "",
    );
    setEditMonitorType(
      monitorTypeSelectValue(editRequest.facet, editRequest.requestedMonitorType),
    );
  }, [editProfileOptions, editRequest]);

  const openApprovalDialog = (request: MediaRequestRecord) => {
    onLoadQualityProfileOptions();
    setApprovalRequest(request);
  };

  const closeApprovalDialog = () => {
    setApprovalRequest(null);
    setApprovalProfileId("");
    setApprovalMonitorType("futureEpisodes");
  };

  const confirmApproval = () => {
    if (!approvalRequest || !approvalProfileId) return;
    onApprove(approvalRequest, {
      qualityProfileId: approvalProfileId,
      monitorType:
        approvalRequest.facet === "movie" ? undefined : approvalMonitorType,
    });
    closeApprovalDialog();
  };

  const openEditDialog = (request: MediaRequestRecord) => {
    onLoadQualityProfileOptions();
    setEditRequest(request);
  };

  const closeEditDialog = () => {
    setEditRequest(null);
    setEditProfileId("");
    setEditMonitorType("futureEpisodes");
  };

  const confirmUpdate = () => {
    if (!editRequest || !editProfileId) return;
    onUpdateRequest(editRequest, {
      requestedQualityProfileId: editProfileId,
      requestedMonitorType: editRequest.facet === "movie" ? undefined : editMonitorType,
    });
    closeEditDialog();
  };

  return (
    <section className="scry-scroll flex min-h-0 flex-1 overflow-y-auto bg-[var(--scry-surfE)]">
      <div className="mx-auto flex w-full max-w-[1240px] flex-col gap-4 px-4 py-6 sm:px-6 lg:px-8">
        <div className="flex items-start gap-4">
          <div className="flex h-11 w-11 flex-none items-center justify-center rounded-[13px] border border-[var(--scry-baccent)] bg-[rgba(var(--scry-accent-rgb),0.16)] text-[var(--scry-accent-text)]">
            <HeadingIcon className="h-5 w-5" />
          </div>
          <div className="min-w-0 flex-1">
            <h1 className="font-display text-[25px] font-bold leading-tight text-[var(--scry-ink)]">
              {headingTitle}
            </h1>
            <p className="mt-1 max-w-2xl text-[13.5px] text-[var(--scry-muted)]">
              {headingCopy}
            </p>
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-3">
          {showModeSwitch ? (
            <div className="inline-flex h-10 rounded-[9px] border border-border bg-background p-1">
              <Button
                id="requests-mode-admin"
                type="button"
                variant={mode === "admin" ? "default" : "ghost"}
                size="sm"
                className="h-8 rounded-[7px]"
                onClick={() => onModeChange("admin")}
              >
                {t("requests.mode.admin")}
              </Button>
              <Button
                id="requests-mode-mine"
                type="button"
                variant={mode === "mine" ? "default" : "ghost"}
                size="sm"
                className="h-8 rounded-[7px]"
                onClick={() => onModeChange("mine")}
              >
                {t("requests.mode.mine")}
              </Button>
            </div>
          ) : null}
          <LibraryMultiSelect
            libraries={libraries}
            selectedLibraryIds={selectedLibraryIds}
            onSelectedLibraryIdsChange={onSelectedLibraryIdsChange}
            triggerClassName="h-10 min-w-56 rounded-[11px]"
          />
          {mode === "mine" ? (
            <Button
              type="button"
              variant="outline"
              className="h-10 w-10 rounded-[11px] p-0"
              onClick={onRefresh}
              disabled={loading}
              aria-label="Refresh requests"
            >
              <RefreshCw className={cn("h-4 w-4", loading && "animate-spin")} />
            </Button>
          ) : null}
          <div className="ml-auto flex flex-wrap items-center gap-2">
            {mode === "admin"
              ? ([
                  ["movie", t("search.facetMovies")],
                  ["series", t("search.facetSeries")],
                  ["anime", t("search.facetAnime")],
                ] as Array<[RequestFacetFilter, string]>).map(([facet, label]) => (
                  <Button
                    key={facet}
                    type="button"
                    size="sm"
                    aria-pressed={adminFacetFilters[facet]}
                    variant={adminFacetFilters[facet] ? "default" : "outline"}
                    className="h-9 rounded-[9px] px-3 text-xs font-semibold"
                    onClick={() =>
                      setAdminFacetFilters((current) => ({
                        ...current,
                        [facet]: !current[facet],
                      }))
                    }
                  >
                    {label}
                    <span className="ml-1 opacity-80">
                      {requestCountByFacet(requests, facet)}
                    </span>
                  </Button>
                ))
              : null}
            {filters.map((filter) => (
              <Button
                key={filter.value}
                type="button"
                size="sm"
                variant={statusFilter === filter.value ? "default" : "outline"}
                className="h-9 rounded-[9px] px-3 text-xs font-semibold"
                onClick={() => onStatusFilterChange(filter.value)}
              >
                {filter.label}
              </Button>
            ))}
          </div>
        </div>

        <div className="grid gap-3">
        {displayedRequests.length === 0 && !loading ? (
          <div
            id={mode === "admin" ? "requests-empty-admin" : "requests-empty-mine"}
            className="rounded-lg border border-dashed border-border bg-card/40 px-4 py-8 text-center text-sm text-muted-foreground"
          >
            {mode === "admin" ? t("requests.empty") : t("requests.emptyMine")}
          </div>
        ) : null}

        {mode === "admin" && displayedRequests.length > 0 ? (
          <div className="overflow-hidden rounded-[13px] border border-[var(--scry-border)] bg-[var(--scry-surf)]">
            <div className="overflow-x-auto">
              <table className="min-w-[920px] w-full border-collapse text-left text-sm">
                <thead className="border-b border-[var(--scry-border2)] bg-[var(--scry-inset)] text-[11px] font-bold uppercase tracking-[0.06em] text-[var(--scry-faint2)]">
                  <tr>
                    <th className="px-4 py-3">Title</th>
                    <th className="px-3 py-3">Requester</th>
                    <th className="px-3 py-3">Library</th>
                    <th className="px-3 py-3">Quality</th>
                    <th className="px-3 py-3">Updated</th>
                    <th className="px-3 py-3">Status</th>
                    <th className="px-4 py-3 text-right">Actions</th>
                  </tr>
                </thead>
                <tbody>
                  {displayedRequests.map((request) => {
                    const posterUrl = selectPosterVariantUrl(request.posterUrl, "w70");
                    const requesters = requesterLabel(request);
                    const externalIds = requestExternalIdLabel(request);
                    const isResolving = actionRequestId === request.id;
                    const approveDisabled = loading || actionRequestId !== null;
                    const actionsDisabled = loading || actionRequestId !== null;
                    const statusMeta = requestStatusTone(t, request.status);
                    const StatusIcon = statusMeta.Icon;
                    const canResolveRequest = request.status === "pending";
                    const libraryLabel =
                      libraries.find((library) => library.id === request.libraryId)
                        ?.name ?? request.libraryId;
                    return (
                      <tr
                        key={request.id}
                        id={mediaRequestRowId(request.id)}
                        data-request-status={request.status}
                        data-request-title={request.title}
                        data-request-facet={request.facet}
                        data-request-imdb-id={requestExternalIdValue(request, "imdb")}
                        data-request-tvdb-id={requestExternalIdValue(request, "tvdb")}
                        data-request-tmdb-id={requestExternalIdValue(request, "tmdb")}
                        className="border-b border-[var(--scry-line2)] last:border-b-0 hover:bg-[var(--scry-hover)]"
                      >
                        <td className="px-4 py-3">
                          <div className="flex min-w-0 items-center gap-3">
                            <div className="h-12 w-8 flex-none overflow-hidden rounded-[6px] border border-border bg-muted">
                              {posterUrl ? (
                                <TitlePoster
                                  src={posterUrl}
                                  alt={t("media.posterAlt", { name: request.title })}
                                  className="h-full w-full object-cover"
                                  loading="lazy"
                                />
                              ) : (
                                <div className="flex h-full w-full items-center justify-center text-[9px] text-muted-foreground">
                                  {t("label.noArt")}
                                </div>
                              )}
                            </div>
                            <div className="min-w-0">
                              <div className="truncate font-semibold text-[var(--scry-ink)]">
                                {request.title}
                              </div>
                              <div className="truncate text-xs text-muted-foreground">
                                {request.year ?? t("label.yearUnknown")}
                                {externalIds ? ` - ${externalIds}` : ""}
                              </div>
                            </div>
                          </div>
                        </td>
                        <td className="px-3 py-3 text-xs text-[var(--scry-body)]">
                          <span className="inline-flex items-center gap-1.5">
                            <RequesterAvatarStack request={request} />
                            <span>{requesters || t("label.unknown")}</span>
                          </span>
                        </td>
                        <td className="px-3 py-3 text-xs text-muted-foreground">
                          {libraryLabel}
                        </td>
                        <td className="px-3 py-3 text-xs text-muted-foreground">
                          {profileLabel(
                            request.requestedQualityProfileId,
                            request.requestedQualityProfileName,
                            qualityProfileOptions,
                          ) ?? t("requests.libraryDefaultProfile")}
                        </td>
                        <td className="px-3 py-3 text-xs text-muted-foreground">
                          {formatUiDateTime(request.updatedAt, dateTimeFormat)}
                        </td>
                        <td className="px-3 py-3">
                          <span
                            id={mediaRequestStatusId(request.id)}
                            data-request-status={request.status}
                            className={cn(
                              "inline-flex items-center gap-1.5 rounded-[7px] border px-2 py-1 text-[11px] font-bold uppercase",
                              statusMeta.className,
                            )}
                          >
                            <StatusIcon className="h-3 w-3" />
                            {statusMeta.label}
                          </span>
                        </td>
                        <td className="px-4 py-3">
                          <div className="flex justify-end gap-2">
                            {canResolveRequest ? (
                              <>
                                <Button
                                  id={mediaRequestApproveId(request.id)}
                                  type="button"
                                  size="sm"
                                  className="h-8"
                                  onClick={() => openApprovalDialog(request)}
                                  disabled={approveDisabled}
                                >
                                  {isResolving ? (
                                    <Loader2 className="h-4 w-4 animate-spin" />
                                  ) : (
                                    <Check className="h-4 w-4" />
                                  )}
                                  {t("requests.approve")}
                                </Button>
                                <Button
                                  id={mediaRequestDismissId(request.id)}
                                  type="button"
                                  size="sm"
                                  variant="outline"
                                  className="h-8"
                                  onClick={() => onDismiss(request)}
                                  disabled={actionsDisabled}
                                >
                                  <X className="h-4 w-4" />
                                  {t("requests.dismiss")}
                                </Button>
                              </>
                            ) : (
                              <span className="text-xs text-muted-foreground">-</span>
                            )}
                          </div>
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          </div>
        ) : null}

        {mode === "mine" ? displayedRequests.map((request) => {
          const posterUrl = selectPosterVariantUrl(request.posterUrl, "w70");
          const requesters = requesterLabel(request);
          const externalIds = requestExternalIdLabel(request);
          const isResolving = actionRequestId === request.id;
          const actionsDisabled = loading || actionRequestId !== null;
          const statusMeta = requestStatusTone(t, request.status);
          const StatusIcon = statusMeta.Icon;
          const canEditOwnRequest = mode === "mine" && request.status === "pending";
          return (
            <article
              key={request.id}
              id={mediaRequestRowId(request.id)}
              data-request-status={request.status}
              data-request-title={request.title}
              data-request-facet={request.facet}
              data-request-imdb-id={requestExternalIdValue(request, "imdb")}
              data-request-tvdb-id={requestExternalIdValue(request, "tvdb")}
              data-request-tmdb-id={requestExternalIdValue(request, "tmdb")}
              className="rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-surf)] p-4"
            >
              <div className="flex gap-4">
                <div className="h-[90px] w-[60px] flex-none overflow-hidden rounded-[9px] border border-border bg-muted">
                  {posterUrl ? (
                    <TitlePoster
                      src={posterUrl}
                      alt={t("media.posterAlt", { name: request.title })}
                      className="h-full w-full object-cover"
                      loading="lazy"
                    />
                  ) : (
                    <div className="flex h-full w-full items-center justify-center text-xs text-muted-foreground">
                      {t("label.noArt")}
                    </div>
                  )}
                </div>
                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-start justify-between gap-2">
                    <div className="min-w-0">
                      <h2 className="truncate text-base font-semibold text-foreground">
                        {request.title}
                      </h2>
                      <p className="text-xs text-muted-foreground">
                        {request.year ?? t("label.yearUnknown")}
                        {externalIds ? ` • ${externalIds}` : ""}
                      </p>
                    </div>
                    <div className="flex flex-wrap items-center gap-2">
                      <span
                        id={mediaRequestStatusId(request.id)}
                        data-request-status={request.status}
                        className={cn(
                          "inline-flex items-center gap-1.5 rounded-[7px] border px-2 py-1 text-[11px] font-bold uppercase",
                          statusMeta.className,
                        )}
                      >
                        <StatusIcon className="h-3 w-3" />
                        {statusMeta.label}
                      </span>
                      {canEditOwnRequest ? (
                        <>
                          <Button
                            id={mediaRequestEditId(request.id)}
                            type="button"
                            size="sm"
                            variant="outline"
                            onClick={() => openEditDialog(request)}
                            disabled={actionsDisabled}
                          >
                            <Pencil className="h-4 w-4" />
                            {t("requests.modify")}
                          </Button>
                          <Button
                            id={mediaRequestCancelId(request.id)}
                            type="button"
                            size="sm"
                            variant="outline"
                            onClick={() => onCancelRequest(request)}
                            disabled={actionsDisabled}
                          >
                            {isResolving ? (
                              <Loader2 className="h-4 w-4 animate-spin" />
                            ) : (
                              <X className="h-4 w-4" />
                            )}
                            {t("requests.cancelRequest")}
                          </Button>
                        </>
                      ) : null}
                    </div>
                  </div>
                  {request.overview ? (
                    <p className="mt-2 line-clamp-2 text-sm text-muted-foreground">
                      {request.overview}
                    </p>
                  ) : null}
                  <div className="mt-3 flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
                    <span className="inline-flex items-center gap-1.5">
                      <User className="h-3.5 w-3.5" />
                      {t("requests.requesters")}:{" "}
                      <RequesterAvatarStack request={request} />
                      <span>{requesters || t("label.unknown")}</span>
                    </span>
                    <span className="inline-flex items-center gap-1.5">
                      <Gem className="h-3.5 w-3.5" />
                      {t("requests.requestedQualityProfile")}:{" "}
                      {profileLabel(
                        request.requestedQualityProfileId,
                        request.requestedQualityProfileName,
                        qualityProfileOptions,
                      ) ?? t("requests.libraryDefaultProfile")}
                    </span>
                    {request.requestedMonitorType ? (
                      <span className="inline-flex items-center gap-1.5">
                        <CircleCheck className="h-3.5 w-3.5" />
                        {t("requests.requestedMonitorType")}:{" "}
                        {monitorTypeLabel(t, request.requestedMonitorType)}
                      </span>
                    ) : null}
                    <span className="inline-flex items-center gap-1.5">
                      <History className="h-3.5 w-3.5" />
                      {t("requests.updated")}:{" "}
                      {formatUiDateTime(request.updatedAt, dateTimeFormat)}
                    </span>
                  </div>
                </div>
              </div>
            </article>
          );
        }) : null}
      </div>
      </div>
      <Dialog open={approvalRequest !== null} onOpenChange={(open) => { if (!open) closeApprovalDialog(); }}>
        <DialogContent id="approve-media-request-dialog" className="sm:max-w-sm">
          <DialogHeader>
            <DialogTitle>{t("requests.approveTitle")}</DialogTitle>
          </DialogHeader>
          <label className="space-y-2">
            <span className="block text-sm font-medium text-card-foreground">
              {t("requests.approvedQualityProfile")}
            </span>
            <Select
              value={approvalProfileId}
              onValueChange={setApprovalProfileId}
              disabled={loading || actionRequestId !== null}
            >
              <SelectTrigger id="approve-media-request-quality-profile">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {qualityProfileOptions.map((profile) => (
                  <SelectItem
                    id={mediaRequestProfileOptionId("approve", profile.id)}
                    key={profile.id}
                    value={profile.id}
                  >
                    {profile.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </label>
          {approvalRequest && approvalRequest.facet !== "movie" ? (
            <label className="space-y-2">
              <span className="block text-sm font-medium text-card-foreground">
                {t("requests.approvedMonitorType")}
              </span>
              <Select
                value={approvalMonitorType}
                onValueChange={(value) =>
                  setApprovalMonitorType(value as RequestMonitorType)
                }
                disabled={loading || actionRequestId !== null}
              >
                <SelectTrigger
                  id="approve-media-request-monitor-type"
                  className="w-full"
                >
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {monitorOptions(t).map((option) => (
                    <SelectItem
                      id={mediaRequestMonitorOptionId("approve", option.value)}
                      key={option.value}
                      value={option.value}
                    >
                      {option.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </label>
          ) : null}
          <DialogFooter>
            <Button id="approve-media-request-cancel" type="button" variant="outline" onClick={closeApprovalDialog}>
              {t("label.cancel")}
            </Button>
            <Button
              id="approve-media-request-confirm"
              type="button"
              onClick={confirmApproval}
              disabled={!approvalProfileId || loading || actionRequestId !== null}
            >
              <Check className="h-4 w-4" />
              {t("requests.approve")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <Dialog open={editRequest !== null} onOpenChange={(open) => { if (!open) closeEditDialog(); }}>
        <DialogContent id="edit-media-request-dialog" className="sm:max-w-sm">
          <DialogHeader>
            <DialogTitle>{t("requests.modifyTitle")}</DialogTitle>
          </DialogHeader>
          <label className="space-y-2">
            <span className="block text-sm font-medium text-card-foreground">
              {t("requests.requestedQualityProfile")}
            </span>
            <Select
              value={editProfileId}
              onValueChange={setEditProfileId}
              disabled={loading || actionRequestId !== null || editProfileOptions.length === 0}
            >
              <SelectTrigger id="edit-media-request-quality-profile">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {editProfileOptions.map((profile) => (
                  <SelectItem
                    id={mediaRequestProfileOptionId("edit", profile.id)}
                    key={profile.id}
                    value={profile.id}
                  >
                    {profile.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </label>
          {editRequest && editRequest.facet !== "movie" ? (
            <label className="space-y-2">
              <span className="block text-sm font-medium text-card-foreground">
                {t("requests.requestedMonitorType")}
              </span>
              <Select
                value={editMonitorType}
                onValueChange={(value) => setEditMonitorType(value as RequestMonitorType)}
                disabled={loading || actionRequestId !== null}
              >
                <SelectTrigger id="edit-media-request-monitor-type">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {monitorOptions(t).map((option) => (
                    <SelectItem
                      id={mediaRequestMonitorOptionId("edit", option.value)}
                      key={option.value}
                      value={option.value}
                    >
                      {option.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </label>
          ) : null}
          <DialogFooter>
            <Button id="edit-media-request-cancel" type="button" variant="outline" onClick={closeEditDialog}>
              {t("label.cancel")}
            </Button>
            <Button
              id="edit-media-request-confirm"
              type="button"
              onClick={confirmUpdate}
              disabled={!editProfileId || loading || actionRequestId !== null}
            >
              <Check className="h-4 w-4" />
              {t("requests.saveChanges")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  );
}
