import {
  AlertTriangle,
  ArrowDownToLine,
  Ban,
  ArchiveRestore,
  FileEdit,
  HardDrive,
  Replace,
  RefreshCcw,
  SkipForward,
  Trash2,
  XCircle,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";

export const TITLE_HISTORY_FILTERS = [
  "grabbed",
  "download_failed",
  "blocklisted",
  "imported",
  "import_failed",
  "import_skipped",
  "file_upgraded",
  "file_recycled",
  "file_deleted",
  "file_renamed",
  "rematched",
] as const;

export const WANTED_HISTORY_FILTERS = [
  "grabbed",
  "download_failed",
  "blocklisted",
  "imported",
  "import_failed",
  "import_skipped",
] as const;

type EventMeta = {
  icon: LucideIcon;
  iconClassName: string;
  labelKey: string;
  badgeClassName: string;
};

const eventMeta: Record<string, EventMeta> = {
  grabbed: {
    icon: ArrowDownToLine,
    iconClassName: "text-sky-400",
    labelKey: "history.grabbed",
    badgeClassName: "border-sky-500/40 bg-sky-500/10 text-sky-200",
  },
  download_failed: {
    icon: AlertTriangle,
    iconClassName: "text-rose-400",
    labelKey: "history.downloadFailed",
    badgeClassName: "border-rose-500/40 bg-rose-500/10 text-rose-200",
  },
  blocklisted: {
    icon: Ban,
    iconClassName: "text-amber-400",
    labelKey: "history.blocklisted",
    badgeClassName: "border-amber-500/40 bg-amber-500/10 text-amber-200",
  },
  imported: {
    icon: HardDrive,
    iconClassName: "text-emerald-400",
    labelKey: "history.imported",
    badgeClassName: "border-emerald-500/40 bg-emerald-500/10 text-emerald-200",
  },
  import_failed: {
    icon: XCircle,
    iconClassName: "text-rose-400",
    labelKey: "history.importFailed",
    badgeClassName: "border-rose-500/40 bg-rose-500/10 text-rose-200",
  },
  import_skipped: {
    icon: SkipForward,
    iconClassName: "text-amber-400",
    labelKey: "history.importSkipped",
    badgeClassName: "border-amber-500/40 bg-amber-500/10 text-amber-200",
  },
  file_upgraded: {
    icon: Replace,
    iconClassName: "text-emerald-400",
    labelKey: "history.fileUpgraded",
    badgeClassName: "border-emerald-500/40 bg-emerald-500/10 text-emerald-200",
  },
  file_recycled: {
    icon: ArchiveRestore,
    iconClassName: "text-amber-400",
    labelKey: "history.fileRecycled",
    badgeClassName: "border-amber-500/40 bg-amber-500/10 text-amber-200",
  },
  file_deleted: {
    icon: Trash2,
    iconClassName: "text-rose-400",
    labelKey: "history.fileDeleted",
    badgeClassName: "border-rose-500/40 bg-rose-500/10 text-rose-200",
  },
  file_renamed: {
    icon: FileEdit,
    iconClassName: "text-cyan-400",
    labelKey: "history.fileRenamed",
    badgeClassName: "border-cyan-500/40 bg-cyan-500/10 text-cyan-200",
  },
  rematched: {
    icon: RefreshCcw,
    iconClassName: "text-violet-400",
    labelKey: "history.rematched",
    badgeClassName: "border-violet-500/40 bg-violet-500/10 text-violet-200",
  },
};

const fallbackMeta: EventMeta = {
  icon: HardDrive,
  iconClassName: "text-muted-foreground",
  labelKey: "history.unknownEvent",
  badgeClassName: "border-border bg-muted text-card-foreground",
};

export function getTitleHistoryEventMeta(eventType: string): EventMeta {
  return eventMeta[eventType] ?? fallbackMeta;
}

export function getTitleHistoryEventLabel(
  eventType: string,
  translate: (key: string) => string,
): string {
  return translate(getTitleHistoryEventMeta(eventType).labelKey);
}

export function getTitleHistoryFilterLabel(
  eventType: string,
  translate: (key: string) => string,
): string {
  return getTitleHistoryEventLabel(eventType, translate);
}
