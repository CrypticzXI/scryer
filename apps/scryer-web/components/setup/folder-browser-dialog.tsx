import { useState, useCallback, useEffect } from "react";
import { useClient } from "urql";
import { Folder, FolderOpen, ChevronRight, ArrowUp, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import { browsePathQuery } from "@/lib/graphql/queries";
import { cn } from "@/lib/utils";
import { selectorId } from "@/lib/utils/dom-ids";

interface DirectoryEntry {
  name: string;
  path: string;
}

interface FolderBrowserDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSelect: (path: string) => void;
  initialPath?: string;
  title?: string;
}

export function FolderBrowserDialog({
  open,
  onOpenChange,
  onSelect,
  initialPath = "/",
  title = "Select folder",
}: FolderBrowserDialogProps) {
  const client = useClient();
  const [currentPath, setCurrentPath] = useState(initialPath || "/");
  const [entries, setEntries] = useState<DirectoryEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [browsedPath, setBrowsedPath] = useState<string | null>(null);

  const browse = useCallback(
    async (path: string) => {
      const nextPath = path.trim() || "/";
      setCurrentPath(nextPath);
      setLoading(true);
      setError(null);
      const { data, error: gqlError } = await client
        .query(browsePathQuery, { path: nextPath })
        .toPromise();
      setLoading(false);
      if (gqlError) {
        setEntries([]);
        setBrowsedPath(null);
        setError(userFacingGraphQlErrorMessage(gqlError, "Unable to browse folder."));
        return;
      }
      setEntries(data?.browsePath ?? []);
      setBrowsedPath(nextPath);
    },
    [client],
  );

  useEffect(() => {
    if (open) {
      browse(initialPath || "/");
    }
  }, [open, initialPath, browse]);

  const parentPath = currentPath === "/" ? null : currentPath.replace(/\/[^/]+\/?$/, "") || "/";

  const pathSegments = currentPath.split("/").filter(Boolean);
  const canSelect = !loading && error === null && browsedPath === currentPath;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        id="folder-browser-dialog"
        className="overflow-hidden border-[var(--scry-border)] bg-[var(--scry-surf)] p-0 text-[var(--scry-ink2)] shadow-[0_24px_80px_rgba(0,0,0,0.55)] sm:max-w-2xl"
      >
        <DialogHeader className="border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,rgba(255,255,255,0.04),rgba(255,255,255,0))] px-4 py-3 sm:px-5">
          <DialogTitle className="flex items-center gap-2 text-[15px] font-semibold text-[var(--scry-ink2)]">
            <span className="grid h-8 w-8 place-items-center rounded-[10px] border border-[var(--scry-border2)] bg-[var(--scry-card2)] text-[var(--scry-accent-text)]">
              <FolderOpen className="h-4 w-4" />
            </span>
            {title}
          </DialogTitle>
          <DialogDescription className="sr-only">
            Browse folders on the Scryer host and select the current path.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-3 p-4 sm:p-5">
          <div className="rounded-[12px] border border-[var(--scry-line2)] bg-[var(--scry-card2)] p-3">
            <div className="flex items-center gap-1 overflow-x-auto pb-1 text-sm">
              <button
                type="button"
                onClick={() => browse("/")}
                className={cn(
                  "shrink-0 rounded-[8px] border px-2 py-1 font-mono transition-colors",
                  pathSegments.length === 0
                    ? "border-[var(--scry-baccent)] bg-[rgba(var(--scry-accent-rgb),0.14)] text-[var(--scry-accent-text)]"
                    : "border-transparent text-[var(--scry-muted3)] hover:border-[var(--scry-border3)] hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)]",
                )}
              >
                /
              </button>
              {pathSegments.map((segment, i) => {
                const segPath = "/" + pathSegments.slice(0, i + 1).join("/");
                const isLast = i === pathSegments.length - 1;
                return (
                  <span key={segPath} className="flex items-center gap-1">
                    <ChevronRight className="h-3 w-3 shrink-0 text-[var(--scry-muted3)]" />
                    <button
                      type="button"
                      onClick={() => browse(segPath)}
                      className={cn(
                        "shrink-0 rounded-[8px] border px-2 py-1 transition-colors",
                        isLast
                          ? "border-[var(--scry-baccent)] bg-[rgba(var(--scry-accent-rgb),0.14)] font-medium text-[var(--scry-accent-text)]"
                          : "border-transparent text-[var(--scry-muted3)] hover:border-[var(--scry-border3)] hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)]",
                      )}
                    >
                      {segment}
                    </button>
                  </span>
                );
              })}
            </div>
            <div className="mt-2 flex gap-2">
              <Input
                id="folder-browser-path-input"
                value={currentPath}
                onChange={(e) => {
                  setCurrentPath(e.target.value);
                  setEntries([]);
                  setError(null);
                  setBrowsedPath(null);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") browse(currentPath);
                }}
                className="h-10 border-[var(--scry-border3)] bg-[var(--scry-inset)] font-mono text-sm text-[var(--scry-ink2)]"
              />
              <Button
                id="folder-browser-go"
                variant="secondary"
                size="sm"
                className="h-10 shrink-0 border-[var(--scry-border3)]"
                onClick={() => browse(currentPath)}
              >
                Go
              </Button>
            </div>
          </div>

          <div className="h-[22rem] overflow-hidden rounded-[12px] border border-[var(--scry-line2)] bg-[var(--scry-card2)]">
            {loading ? (
              <div className="flex h-full items-center justify-center">
                <div className="flex items-center gap-3 rounded-[12px] border border-[var(--scry-border2)] bg-[var(--scry-inset)] px-4 py-3 text-sm text-[var(--scry-muted3)]">
                  <Loader2 className="h-5 w-5 animate-spin text-[var(--scry-accent-text)]" />
                  Loading folders
                </div>
              </div>
            ) : error ? (
              <div className="flex h-full flex-col items-center justify-center gap-3 px-6 text-center text-sm">
                <p className="max-w-md rounded-[10px] border border-rose-500/40 bg-rose-500/10 px-3 py-2 text-rose-700 dark:text-rose-300">
                  {error}
                </p>
                {currentPath !== "/" ? (
                  <Button
                    id="folder-browser-root-recovery"
                    type="button"
                    variant="secondary"
                    size="sm"
                    onClick={() => browse("/")}
                  >
                    Browse /
                  </Button>
                ) : null}
              </div>
            ) : (
              <div className="h-full overflow-y-auto p-2">
                {parentPath !== null && (
                  <button
                    id="folder-browser-up"
                    type="button"
                    onClick={() => browse(parentPath)}
                    className="mb-1 flex w-full items-center gap-2.5 rounded-[10px] border border-transparent px-3 py-2 text-left text-sm text-[var(--scry-muted3)] transition-colors hover:border-[var(--scry-border3)] hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)]"
                  >
                    <span className="grid h-8 w-8 place-items-center rounded-[9px] bg-[var(--scry-inset)]">
                      <ArrowUp className="h-4 w-4 shrink-0" />
                    </span>
                    <span>..</span>
                  </button>
                )}
                {entries.length === 0 && !loading && (
                  <div className="px-3 py-10 text-center text-sm text-[var(--scry-muted3)]">
                    No subdirectories
                  </div>
                )}
                {entries.map((entry) => (
                  <button
                    id={selectorId("folder-browser-entry", entry.path)}
                    key={entry.path}
                    type="button"
                    onClick={() => browse(entry.path)}
                    className="mb-1 flex w-full items-center gap-2.5 rounded-[10px] border border-transparent px-3 py-2 text-left text-sm transition-colors hover:border-[var(--scry-border3)] hover:bg-[var(--scry-hover)]"
                  >
                    <span className="grid h-8 w-8 place-items-center rounded-[9px] bg-[var(--scry-inset)] text-[var(--scry-accent-text)]">
                      <Folder className="h-4 w-4 shrink-0" />
                    </span>
                    <span className="truncate text-[var(--scry-ink2)]">{entry.name}</span>
                  </button>
                ))}
              </div>
            )}
          </div>
        </div>

        <DialogFooter className="border-t border-[var(--scry-border3)] bg-[var(--scry-inset)] px-4 py-3 sm:flex-row sm:items-center sm:justify-between sm:px-5">
          <p className="min-w-0 truncate text-left font-mono text-xs text-[var(--scry-muted3)] sm:mr-auto">
            {currentPath}
          </p>
          <div className="flex shrink-0 justify-end gap-2">
          <Button id="folder-browser-cancel" variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button
            id="folder-browser-select"
            disabled={!canSelect}
            onClick={() => {
              onSelect(currentPath);
              onOpenChange(false);
            }}
          >
            <FolderOpen className="mr-1.5 h-4 w-4" />
            Select folder
          </Button>
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
