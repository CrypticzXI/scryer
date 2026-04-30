import * as React from "react";
import { FolderOpen, Plus, Trash2 } from "lucide-react";
import { FolderBrowserDialog } from "@/components/setup/folder-browser-dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useTranslate } from "@/lib/context/translate-context";

type PathMappingRow = {
  localPath: string;
  remotePath: string;
};

type LocalRemotePathMappingsFieldProps = {
  fieldKey: string;
  label: string;
  value: string;
  helpText?: string | null;
  required?: boolean;
  maxRows?: number;
  onChange: (key: string, value: string) => void;
};

const DEFAULT_MAX_ROWS = 10;

function emptyRow(): PathMappingRow {
  return { localPath: "", remotePath: "" };
}

function parsePathMappings(value: string): PathMappingRow[] {
  const rows = value
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.length > 0)
    .map((line) => {
      const [localPath, remotePath = ""] = line.split(/=>/, 2);
      return {
        localPath: localPath.trim(),
        remotePath: remotePath.trim(),
      };
    });

  return rows.length > 0 ? rows : [emptyRow()];
}

function serializePathMappings(rows: PathMappingRow[]): string {
  return rows
    .filter((row) => row.localPath.trim() || row.remotePath.trim())
    .map((row) => `${row.localPath.trim()} => ${row.remotePath.trim()}`)
    .join("\n");
}

export function LocalRemotePathMappingsField({
  fieldKey,
  label,
  value,
  helpText,
  required = false,
  maxRows = DEFAULT_MAX_ROWS,
  onChange,
}: LocalRemotePathMappingsFieldProps) {
  const t = useTranslate();
  const [rows, setRows] = React.useState<PathMappingRow[]>(() =>
    parsePathMappings(value),
  );
  const [browseRowIndex, setBrowseRowIndex] = React.useState<number | null>(null);

  React.useEffect(() => {
    setRows((currentRows) =>
      serializePathMappings(currentRows) === value
        ? currentRows
        : parsePathMappings(value),
    );
  }, [value]);

  const writeRows = React.useCallback(
    (nextRows: PathMappingRow[]) => {
      const normalizedRows = nextRows.length > 0 ? nextRows : [emptyRow()];
      setRows(normalizedRows);
      onChange(fieldKey, serializePathMappings(normalizedRows));
    },
    [fieldKey, onChange],
  );

  const updateRow = React.useCallback(
    (index: number, patch: Partial<PathMappingRow>) => {
      writeRows(
        rows.map((row, rowIndex) =>
          rowIndex === index ? { ...row, ...patch } : row,
        ),
      );
    },
    [rows, writeRows],
  );

  const removeRow = React.useCallback(
    (index: number) => {
      writeRows(rows.filter((_, rowIndex) => rowIndex !== index));
    },
    [rows, writeRows],
  );

  const addRow = React.useCallback(() => {
    if (rows.length >= maxRows) {
      return;
    }
    writeRows([...rows, emptyRow()]);
  }, [maxRows, rows, writeRows]);

  const handleFolderSelect = React.useCallback(
    (path: string) => {
      if (browseRowIndex == null) {
        return;
      }
      updateRow(browseRowIndex, { localPath: path });
      setBrowseRowIndex(null);
    },
    [browseRowIndex, updateRow],
  );

  const browseInitialPath =
    browseRowIndex == null ? "/" : (rows[browseRowIndex]?.localPath.trim() || "/");
  const isEmpty = serializePathMappings(rows).trim().length === 0;

  return (
    <label className="block">
      <Label className="mb-2 block">{label}</Label>
      <div className="space-y-2">
        {rows.map((row, index) => (
          <div
            key={index}
            className="grid min-w-0 grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)_auto] items-center gap-2"
          >
            <div className="relative min-w-0">
              <Input
                value={row.localPath}
                readOnly
                onClick={() => setBrowseRowIndex(index)}
                className="cursor-pointer pr-10 font-mono text-sm"
                aria-label={`${label} ${index + 1}`}
              />
              <Button
                type="button"
                variant="ghost"
                size="icon-sm"
                className="absolute right-1 top-1/2 h-7 w-7 -translate-y-1/2"
                onClick={() => setBrowseRowIndex(index)}
                title={t("setup.browse")}
                aria-label={t("setup.browse")}
              >
                <FolderOpen className="h-4 w-4" />
              </Button>
            </div>

            <span
              aria-hidden="true"
              className="shrink-0 px-1 text-sm text-muted-foreground [font-variant-ligatures:none]"
            >
              =&gt;
            </span>

            <Input
              value={row.remotePath}
              onChange={(event) => updateRow(index, { remotePath: event.target.value })}
              required={required && isEmpty && index === 0}
              className="font-mono text-sm"
            />

            {rows.length > 1 ? (
              <Button
                type="button"
                variant="ghost"
                size="icon-sm"
                className="h-9 w-9 shrink-0"
                onClick={() => removeRow(index)}
                title={t("label.remove")}
                aria-label={t("label.remove")}
              >
                <Trash2 className="h-4 w-4" />
              </Button>
            ) : (
              <div className="h-9 w-9 shrink-0" aria-hidden="true" />
            )}
          </div>
        ))}

        <Button
          type="button"
          variant="outline"
          size="icon"
          className="h-10 w-10"
          onClick={addRow}
          disabled={rows.length >= maxRows}
          title={t("label.add")}
          aria-label={t("label.add")}
        >
          <Plus className="h-5 w-5" />
        </Button>

        {helpText ? (
          <p className="text-xs text-muted-foreground">{helpText}</p>
        ) : null}
      </div>

      <FolderBrowserDialog
        open={browseRowIndex != null}
        onOpenChange={(open) => {
          if (!open) {
            setBrowseRowIndex(null);
          }
        }}
        onSelect={handleFolderSelect}
        initialPath={browseInitialPath}
        title={label}
      />
    </label>
  );
}
