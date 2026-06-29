import type { CSSProperties, DragEvent } from "react";
import {
  ArrowLeftRight,
  ArrowRight,
  FolderInput,
  GripVertical,
  X,
} from "lucide-react";

import {
  effectiveRootPath,
  isRootRemapped,
  type ImportRoot,
} from "@/lib/hooks/use-external-import-setup";

import { ImportInstancePill } from "./import-instance-pill";

interface ImportRootChipProps {
  root: ImportRoot;
  variant: "tray" | "library";
  isMobile: boolean;
  onRemap?: () => void;
  onAssign?: () => void;
  onRemoveManual?: () => void;
  onManualPathChange?: (path: string) => void;
  draggable?: boolean;
  onDragStart?: (e: DragEvent) => void;
  onDragEnd?: (e: DragEvent) => void;
  t: (key: string, values?: Record<string, unknown>) => string;
}

const MONO: CSSProperties = {
  fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
};

/**
 * Draggable source-root chip. Two variants:
 *  - "tray": the Source Roots tray chip (taller, full provenance display).
 *  - "library": the compact chip shown inside a library card's drop body.
 * Detected roots show their effective Scryer path (struck-through provenance
 * when remapped) and a Remap control; manual roots show an editable path input
 * and a delete button (no Remap).
 */
export function ImportRootChip({
  root,
  variant,
  isMobile,
  onRemap,
  onAssign,
  onRemoveManual,
  onManualPathChange,
  draggable,
  onDragStart,
  onDragEnd,
  t,
}: ImportRootChipProps) {
  const library = variant === "library";
  const remapped = isRootRemapped(root);
  const effective = effectiveRootPath(root);
  const pathTitle = remapped
    ? `${t("setup.provenanceSource")}: ${root.arrRootPath}  →  ${t("setup.provenanceScryer")}: ${effective}`
    : effective;

  return (
    <span
      data-rootchip
      data-slot="import-root-chip"
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: library ? 7 : 8,
        height: library ? 36 : 40,
        padding: "0 8px 0 2px",
        borderRadius: library ? 10 : 11,
        background: "var(--scry-bg)",
        border: "1px solid var(--scry-border2)",
        maxWidth: "100%",
      }}
    >
      {/* Drag grip — desktop only */}
      {!isMobile ? (
        <span
          aria-hidden={!draggable}
          draggable={draggable}
          onDragStart={onDragStart}
          onDragEnd={onDragEnd}
          style={{
            display: "inline-flex",
            alignItems: "center",
            justifyContent: "center",
            width: library ? 20 : 22,
            alignSelf: "stretch",
            cursor: draggable ? "grab" : "default",
            color: "var(--scry-faint3)",
            flex: "none",
          }}
        >
          <GripVertical size={library ? 13 : 14} />
        </span>
      ) : null}

      {/* Assign button — mobile only */}
      {isMobile ? (
        <button
          type="button"
          title={
            library
              ? t("setup.moveToAnotherLibrary")
              : t("setup.assignToLibrary")
          }
          onClick={onAssign}
          style={{
            display: "inline-flex",
            alignItems: "center",
            justifyContent: "center",
            width: library ? 30 : 32,
            height: library ? 30 : 32,
            borderRadius: 8,
            border: "1px solid var(--scry-baccent)",
            background: "rgba(var(--scry-accent-rgb), 0.1)",
            color: "var(--scry-accent-text)",
            flex: "none",
            cursor: "pointer",
          }}
        >
          <FolderInput size={library ? 15 : 16} />
        </button>
      ) : null}

      {/* Instance pill */}
      <ImportInstancePill
        kind={root.kind}
        label={root.instanceLabel}
        title={root.instanceLabel}
        size={library ? "sm" : "md"}
        showDot={root.kind === "manual"}
      />

      {/* Path region */}
      {root.manual ? (
        <ManualPathRegion
          value={root.arrRootPath}
          onChange={onManualPathChange}
          onRemove={onRemoveManual}
          t={t}
        />
      ) : library ? (
        <span
          title={pathTitle}
          style={{
            display: "inline-flex",
            alignItems: "center",
            gap: 4,
            flex: 1,
            minWidth: 0,
          }}
        >
          {remapped ? (
            <ArrowLeftRight
              size={11}
              style={{ color: "var(--scry-accent-text)", flex: "none" }}
            />
          ) : null}
          <span
            style={{
              ...MONO,
              fontSize: 12,
              color: remapped
                ? "var(--scry-accent-text)"
                : "var(--scry-ink2)",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {effective}
          </span>
        </span>
      ) : (
        <span
          title={pathTitle}
          style={{
            display: "inline-flex",
            alignItems: "center",
            gap: 5,
            minWidth: 0,
          }}
        >
          {remapped ? (
            <>
              <span
                style={{
                  ...MONO,
                  fontSize: 11.5,
                  color: "var(--scry-faint3)",
                  textDecoration: "line-through",
                  textDecorationColor: "var(--scry-faint4)",
                  whiteSpace: "nowrap",
                }}
              >
                {root.arrRootPath}
              </span>
              <ArrowRight
                size={12}
                style={{ color: "var(--scry-accent-text)", flex: "none" }}
              />
              <span
                style={{
                  ...MONO,
                  fontSize: 13,
                  color: "var(--scry-accent-text)",
                  whiteSpace: "nowrap",
                }}
              >
                {effective}
              </span>
            </>
          ) : (
            <span
              style={{
                ...MONO,
                fontSize: 13,
                color: "var(--scry-ink2)",
                whiteSpace: "nowrap",
              }}
            >
              {effective}
            </span>
          )}
        </span>
      )}

      {/* Remap button — detected roots only */}
      {!root.manual ? (
        <button
          type="button"
          title={t("setup.remap")}
          onClick={onRemap}
          style={{
            display: "inline-flex",
            alignItems: "center",
            justifyContent: "center",
            height: library ? 28 : 26,
            padding: library ? "0 11px" : "0 10px",
            borderRadius: library ? 8 : 7,
            fontSize: 12,
            fontWeight: 600,
            cursor: "pointer",
            flex: "none",
            background: remapped
              ? "rgba(var(--scry-accent-rgb), 0.1)"
              : "transparent",
            border: `1px solid ${
              remapped ? "var(--scry-baccent)" : "var(--scry-border2)"
            }`,
            color: remapped ? "var(--scry-accent-text)" : "var(--scry-faint)",
          }}
        >
          {t("setup.remap")}
        </button>
      ) : null}
    </span>
  );
}

function ManualPathRegion({
  value,
  onChange,
  onRemove,
  t,
}: {
  value: string;
  onChange?: (path: string) => void;
  onRemove?: () => void;
  t: (key: string, values?: Record<string, unknown>) => string;
}) {
  return (
    <>
      <input
        value={value}
        spellCheck={false}
        placeholder="/path/to/media"
        onChange={(e) => onChange?.(e.target.value)}
        style={{
          ...MONO,
          width: 170,
          height: 28,
          padding: "0 8px",
          borderRadius: 7,
          border: "1px solid var(--scry-border2)",
          background: "var(--scry-page2)",
          color: "var(--scry-ink2)",
          fontSize: 12.5,
          outline: "none",
        }}
      />
      <button
        type="button"
        title={t("setup.deleteLibrary")}
        onClick={onRemove}
        style={{
          display: "inline-flex",
          alignItems: "center",
          justifyContent: "center",
          width: 24,
          height: 24,
          borderRadius: 7,
          border: "1px solid var(--scry-border2)",
          background: "transparent",
          color: "var(--scry-faint)",
          flex: "none",
          cursor: "pointer",
        }}
      >
        <X size={13} />
      </button>
    </>
  );
}
