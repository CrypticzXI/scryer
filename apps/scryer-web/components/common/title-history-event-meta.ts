import {
  AlertTriangle,
  ArrowDownToLine,
  Ban,
  FileEdit,
  HardDrive,
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
  "file_deleted",
  "file_renamed",
  "rematched",
] as const;

type EventMeta = {
  icon: LucideIcon;
  iconClassName: string;
  label: string;
  badgeClassName: string;
  filterLabelKey?: string;
};

const eventMeta: Record<string, EventMeta> = {
  grabbed: {
    icon: ArrowDownToLine,
    iconClassName: "text-sky-400",
    label: "Grabbed",
    badgeClassName: "border-sky-500/40 bg-sky-500/10 text-sky-200",
    filterLabelKey: "history.grabbed",
  },
  download_failed: {
    icon: AlertTriangle,
    iconClassName: "text-rose-400",
    label: "Download Failed",
    badgeClassName: "border-rose-500/40 bg-rose-500/10 text-rose-200",
  },
  blocklisted: {
    icon: Ban,
    iconClassName: "text-amber-400",
    label: "Blocklisted",
    badgeClassName: "border-amber-500/40 bg-amber-500/10 text-amber-200",
  },
  imported: {
    icon: HardDrive,
    iconClassName: "text-emerald-400",
    label: "Imported",
    badgeClassName: "border-emerald-500/40 bg-emerald-500/10 text-emerald-200",
    filterLabelKey: "history.imported",
  },
  import_failed: {
    icon: XCircle,
    iconClassName: "text-rose-400",
    label: "Import Failed",
    badgeClassName: "border-rose-500/40 bg-rose-500/10 text-rose-200",
    filterLabelKey: "history.importFailed",
  },
  import_skipped: {
    icon: SkipForward,
    iconClassName: "text-amber-400",
    label: "Import Skipped",
    badgeClassName: "border-amber-500/40 bg-amber-500/10 text-amber-200",
    filterLabelKey: "history.importSkipped",
  },
  file_deleted: {
    icon: Trash2,
    iconClassName: "text-rose-400",
    label: "Deleted",
    badgeClassName: "border-rose-500/40 bg-rose-500/10 text-rose-200",
    filterLabelKey: "history.fileDeleted",
  },
  file_renamed: {
    icon: FileEdit,
    iconClassName: "text-cyan-400",
    label: "Renamed",
    badgeClassName: "border-cyan-500/40 bg-cyan-500/10 text-cyan-200",
    filterLabelKey: "history.fileRenamed",
  },
  rematched: {
    icon: RefreshCcw,
    iconClassName: "text-violet-400",
    label: "Rematched",
    badgeClassName: "border-violet-500/40 bg-violet-500/10 text-violet-200",
  },
};

const fallbackMeta: EventMeta = {
  icon: HardDrive,
  iconClassName: "text-muted-foreground",
  label: "Unknown",
  badgeClassName: "border-border bg-muted text-card-foreground",
};

export function getTitleHistoryEventMeta(eventType: string): EventMeta {
  return eventMeta[eventType] ?? fallbackMeta;
}

export function getTitleHistoryFilterLabel(
  eventType: string,
  translate: (key: string) => string,
): string {
  const meta = getTitleHistoryEventMeta(eventType);
  return meta.filterLabelKey ? translate(meta.filterLabelKey) : meta.label;
}
