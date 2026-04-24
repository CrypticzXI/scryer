import * as React from "react";
import { useTranslate } from "@/lib/context/translate-context";
import type { SubtitleDownloadRecord } from "@/lib/types/subtitles";

function formatDateTime(value: string) {
  try {
    return new Date(value).toLocaleString();
  } catch {
    return value;
  }
}

function SubtitleFlag({
  label,
  className,
}: {
  label: string;
  className: string;
}) {
  return (
    <span className={`rounded px-1.5 py-0.5 text-[10px] font-medium ${className}`}>
      {label}
    </span>
  );
}

export function ExternalSubtitleSection({
  downloads,
  renderActions,
}: {
  downloads: SubtitleDownloadRecord[];
  renderActions?: (download: SubtitleDownloadRecord) => React.ReactNode;
}) {
  const t = useTranslate();

  return (
    <div className="space-y-2">
      <p className="text-xs font-medium text-muted-foreground">{t("subtitle.external")}</p>
      {downloads.length === 0 ? (
        <p className="text-xs text-muted-foreground/70">
          {t("subtitle.noneDownloaded")}
        </p>
      ) : (
        <div className="space-y-2">
          {downloads.map((download) => (
            <div
              key={download.id}
              className="rounded-lg border border-border/60 bg-background/50 px-3 py-2"
            >
              <div className="flex flex-wrap items-start justify-between gap-2">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="rounded border border-sky-500/30 bg-sky-500/15 px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wide text-sky-700 dark:text-sky-300">
                    {download.language}
                  </span>
                  <span className="rounded border border-border/60 bg-muted/40 px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
                    {download.provider}
                  </span>
                  {download.synced ? (
                    <SubtitleFlag
                      label={t("subtitle.synced")}
                      className="bg-emerald-500/15 text-emerald-700 dark:text-emerald-300"
                    />
                  ) : null}
                  {download.hearingImpaired ? (
                    <SubtitleFlag
                      label={t("subtitle.hearingImpaired")}
                      className="bg-amber-500/20 text-amber-700 dark:text-amber-300"
                    />
                  ) : null}
                  {download.forced ? (
                    <SubtitleFlag
                      label={t("subtitle.forced")}
                      className="bg-purple-500/20 text-purple-700 dark:text-purple-300"
                    />
                  ) : null}
                  {download.aiTranslated ? (
                    <SubtitleFlag
                      label={t("subtitle.aiTranslated")}
                      className="bg-rose-500/20 text-rose-700 dark:text-rose-300"
                    />
                  ) : null}
                  {download.machineTranslated ? (
                    <SubtitleFlag
                      label={t("subtitle.machineTranslated")}
                      className="bg-red-500/20 text-red-700 dark:text-red-300"
                    />
                  ) : null}
                  {download.score != null ? (
                    <span className="text-[11px] text-muted-foreground">
                      {t("subtitle.scoreWithValue", { score: download.score })}
                    </span>
                  ) : null}
                </div>
                {renderActions ? (
                  <div className="flex shrink-0 items-center gap-2">
                    {renderActions(download)}
                  </div>
                ) : null}
              </div>
              <p className="mt-2 break-all font-mono text-[11px] text-muted-foreground">
                {download.filePath}
              </p>
              <div className="mt-1 flex flex-wrap items-center gap-3 text-[11px] text-muted-foreground">
                {download.releaseInfo ? <span>{download.releaseInfo}</span> : null}
                {download.uploader ? <span>{download.uploader}</span> : null}
                <span>{formatDateTime(download.downloadedAt)}</span>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
