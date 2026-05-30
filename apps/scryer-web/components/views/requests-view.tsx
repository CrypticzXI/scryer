import * as React from "react";
import { Check, Loader2, RefreshCw, X } from "lucide-react";

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

type RequestsViewProps = {
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

export function RequestsView({
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
}: RequestsViewProps) {
  const t = useTranslate();
  const [approvalRequest, setApprovalRequest] =
    React.useState<MediaRequestRecord | null>(null);
  const [approvalProfileId, setApprovalProfileId] = React.useState("");

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

  return (
    <section className="flex min-h-0 flex-1 flex-col gap-4">
      <div className="flex flex-wrap items-center gap-2">
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
            {t("requests.empty")}
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
                        {request.status}
                      </span>
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
                    </div>
                  </div>
                  {request.overview ? (
                    <p className="mt-2 line-clamp-2 text-sm text-muted-foreground">
                      {request.overview}
                    </p>
                  ) : null}
                  <div className="mt-3 flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
                    <span>{t("requests.requesters")}: {requesters || t("label.unknown")}</span>
                    <span>
                      {t("requests.requestedQualityProfile")}:{" "}
                      {profileLabel(
                        request.requestedQualityProfileId,
                        request.requestedQualityProfileName,
                        qualityProfileOptions,
                      ) ?? t("requests.libraryDefaultProfile")}
                    </span>
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
    </section>
  );
}
