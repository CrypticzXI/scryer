import * as React from "react";
import { Check, Loader2, Pencil, RefreshCw, X } from "lucide-react";

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
import type { LibraryRecord, MediaRequestRecord } from "@/lib/types";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";
import { cn } from "@/lib/utils";

type QualityProfileOption = {
  id: string;
  name: string;
};

type RequestsMode = "admin" | "mine";

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

type RequestsViewProps = {
  mode: RequestsMode;
  canShowAdminMode: boolean;
  canShowRequesterMode: boolean;
  onModeChange: (mode: RequestsMode) => void;
  libraries: LibraryRecord[];
  selectedLibraryIds: string[];
  onSelectedLibraryIdsChange: (libraryIds: string[]) => void;
  requests: MediaRequestRecord[];
  qualityProfileOptions: QualityProfileOption[];
  loading: boolean;
  actionRequestId: string | null;
  onRefresh: () => void;
  onApprove: (request: MediaRequestRecord, qualityProfileId: string) => void;
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
      return t("requests.status.rejected");
    case "canceled":
      return t("requests.status.canceled");
    default:
      return status;
  }
}

export function RequestsView({
  mode,
  canShowAdminMode,
  canShowRequesterMode,
  onModeChange,
  libraries,
  selectedLibraryIds,
  onSelectedLibraryIdsChange,
  requests,
  qualityProfileOptions,
  loading,
  actionRequestId,
  onRefresh,
  onApprove,
  onDismiss,
  onUpdateRequest,
  onCancelRequest,
}: RequestsViewProps) {
  const t = useTranslate();
  const showModeSwitch = canShowAdminMode && canShowRequesterMode;
  const [approvalRequest, setApprovalRequest] =
    React.useState<MediaRequestRecord | null>(null);
  const [approvalProfileId, setApprovalProfileId] = React.useState("");
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
    setApprovalRequest(request);
  };

  const closeApprovalDialog = () => {
    setApprovalRequest(null);
    setApprovalProfileId("");
  };

  const confirmApproval = () => {
    if (!approvalRequest || !approvalProfileId) return;
    onApprove(approvalRequest, approvalProfileId);
    closeApprovalDialog();
  };

  const openEditDialog = (request: MediaRequestRecord) => {
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
    <section className="flex min-h-0 flex-1 flex-col gap-4">
      <div className="flex flex-wrap items-center gap-2">
        {showModeSwitch ? (
          <div className="inline-flex h-11 rounded-md border border-border bg-background p-1">
            <Button
              type="button"
              variant={mode === "admin" ? "default" : "ghost"}
              size="sm"
              className="h-8"
              onClick={() => onModeChange("admin")}
            >
              {t("requests.mode.admin")}
            </Button>
            <Button
              type="button"
              variant={mode === "mine" ? "default" : "ghost"}
              size="sm"
              className="h-8"
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
          triggerClassName="h-11 min-w-56"
        />
        <Button
          type="button"
          variant="outline"
          className="h-11"
          onClick={onRefresh}
          disabled={loading}
        >
          <RefreshCw className={cn("h-4 w-4", loading && "animate-spin")} />
        </Button>
      </div>

      <div className="grid gap-3">
        {requests.length === 0 && !loading ? (
          <div className="rounded-lg border border-dashed border-border bg-card/40 px-4 py-8 text-center text-sm text-muted-foreground">
            {mode === "admin" ? t("requests.empty") : t("requests.emptyMine")}
          </div>
        ) : null}

        {requests.map((request) => {
          const posterUrl = selectPosterVariantUrl(request.posterUrl, "w70");
          const requesters = requesterLabel(request);
          const externalIds = requestExternalIdLabel(request);
          const isResolving = actionRequestId === request.id;
          const approveDisabled =
            loading || actionRequestId !== null || qualityProfileOptions.length === 0;
          const actionsDisabled = loading || actionRequestId !== null;
          const canEditOwnRequest = mode === "mine" && request.status === "pending";
          return (
            <article
              key={request.id}
              className="rounded-lg border border-border bg-card/60 p-3"
            >
              <div className="flex gap-3">
                <div className="h-24 w-16 flex-none overflow-hidden rounded-md border border-border bg-muted">
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
                      <span className="rounded-md border border-border bg-background px-2 py-1 text-xs font-medium uppercase text-muted-foreground">
                        {requestStatusLabel(t, request.status)}
                      </span>
                      {mode === "admin" ? (
                        <>
                          <Button
                            type="button"
                            size="sm"
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
                            type="button"
                            size="sm"
                            variant="outline"
                            onClick={() => onDismiss(request)}
                            disabled={actionsDisabled}
                          >
                            <X className="h-4 w-4" />
                            {t("requests.dismiss")}
                          </Button>
                        </>
                      ) : canEditOwnRequest ? (
                        <>
                          <Button
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
                      {t("requests.requesters")}:{" "}
                      <RequesterAvatarStack request={request} />
                      <span>{requesters || t("label.unknown")}</span>
                    </span>
                    <span>
                      {t("requests.requestedQualityProfile")}:{" "}
                      {profileLabel(
                        request.requestedQualityProfileId,
                        request.requestedQualityProfileName,
                        qualityProfileOptions,
                      ) ?? t("requests.libraryDefaultProfile")}
                    </span>
                    {request.requestedMonitorType ? (
                      <span>
                        {t("requests.requestedMonitorType")}:{" "}
                        {monitorTypeLabel(t, request.requestedMonitorType)}
                      </span>
                    ) : null}
                    <span>{t("requests.updated")}: {new Date(request.updatedAt).toLocaleString()}</span>
                  </div>
                </div>
              </div>
            </article>
          );
        })}
      </div>
      <Dialog open={approvalRequest !== null} onOpenChange={(open) => { if (!open) closeApprovalDialog(); }}>
        <DialogContent className="sm:max-w-sm">
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
                  <SelectItem key={profile.id} value={profile.id}>
                    {profile.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </label>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={closeApprovalDialog}>
              {t("label.cancel")}
            </Button>
            <Button
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
        <DialogContent className="sm:max-w-sm">
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
                  <SelectItem key={profile.id} value={profile.id}>
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
                    <SelectItem key={option.value} value={option.value}>
                      {option.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </label>
          ) : null}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={closeEditDialog}>
              {t("label.cancel")}
            </Button>
            <Button
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
