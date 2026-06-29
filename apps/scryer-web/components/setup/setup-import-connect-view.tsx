import { useState } from "react";
import type { CSSProperties } from "react";
import {
  CircleAlert,
  CircleCheckBig,
  Info,
  KeyRound,
  Loader2,
  Pencil,
  Plus,
  X,
} from "lucide-react";

import { Input } from "@/components/ui/input";
import { instancePillColors } from "@/components/setup/import/import-instance-pill";
import type {
  ImportInstance,
  ImportInstanceKind,
  UseExternalImportSetupReturn,
} from "@/lib/hooks/use-external-import-setup";

interface SetupImportConnectViewProps {
  wizard: UseExternalImportSetupReturn;
  t: (key: string, values?: Record<string, unknown>) => string;
}

interface KindColumn {
  kind: ImportInstanceKind;
  /** Brand-ish heading accent for the product column (design spec §3). */
  headingColor: string;
  /** Literal brand product name (acceptable per task — these are brand names). */
  productName: string;
  urlPlaceholder: string;
}

const KIND_COLUMNS: readonly KindColumn[] = [
  {
    kind: "sonarr",
    headingColor: "#56b6f0",
    productName: "Sonarr",
    urlPlaceholder: "http://localhost:8989",
  },
  {
    kind: "radarr",
    headingColor: "#e7b94a",
    productName: "Radarr",
    urlPlaceholder: "http://localhost:7878",
  },
  {
    kind: "prowlarr",
    headingColor: "#a987ff",
    productName: "Prowlarr",
    urlPlaceholder: "http://localhost:9696",
  },
];

const FIELD_LABEL_STYLE: CSSProperties = {
  display: "block",
  marginBottom: 5,
  fontSize: 10,
  fontWeight: 700,
  letterSpacing: "0.08em",
  textTransform: "uppercase",
  color: "var(--scry-faint2)",
};

const MONO_INPUT_CLASS =
  "h-[38px] rounded-md font-mono text-[12.5px] text-[#dbe6fb]";

export function SetupImportConnectView({
  wizard,
  t,
}: SetupImportConnectViewProps) {
  const {
    instancesByKind,
    addInstance,
    removeInstance,
    setInstanceName,
    setInstanceConnectionField,
    verifyInstance,
    connectionReady,
  } = wizard;

  /** Fire verification on blur of URL/key — only when the connection is ready. */
  const handleVerifyBlur = (inst: ImportInstance) => {
    if (connectionReady(inst)) void verifyInstance(inst.id);
  };

  return (
    <div data-slot="import-connect-view" className="w-full">
      <div
        data-r="conngrid"
        className="grid items-start gap-4"
        style={{
          marginTop: 26,
          gridTemplateColumns: "repeat(3, minmax(0, 1fr))",
        }}
      >
        {KIND_COLUMNS.map((column) => {
          const instances = instancesByKind(column.kind);
          return (
            <div
              key={column.kind}
              className="flex flex-col gap-3"
              data-slot="import-connect-column"
              data-kind={column.kind}
            >
              {/* Column header — kind-colored product heading (no logo asset). */}
              <div className="flex flex-col items-center gap-2 text-center">
                <span
                  aria-hidden
                  style={{
                    width: 10,
                    height: 10,
                    borderRadius: "50%",
                    background: column.headingColor,
                    boxShadow: `0 0 0 4px ${column.headingColor}22`,
                  }}
                />
                <span
                  style={{
                    fontFamily:
                      "'Space Grotesk', 'Inter Variable', Inter, system-ui, sans-serif",
                    fontWeight: 700,
                    fontSize: 20,
                    color: "#fff",
                  }}
                >
                  {column.productName}
                </span>
              </div>

              {instances.map((inst) => (
                <InstanceCard
                  key={inst.id}
                  inst={inst}
                  urlPlaceholder={column.urlPlaceholder}
                  t={t}
                  onName={(name) => setInstanceName(inst.id, name)}
                  onRemove={() => removeInstance(inst.id)}
                  onField={(field, value) =>
                    setInstanceConnectionField(inst.id, field, value)
                  }
                  onVerifyBlur={() => handleVerifyBlur(inst)}
                />
              ))}

              {/* Add-instance button (dashed, per column). */}
              <button
                type="button"
                onClick={() => addInstance(column.kind)}
                data-slot="import-add-instance"
                className="flex items-center justify-center gap-2 text-[13px] font-semibold transition-colors"
                style={{
                  height: 40,
                  borderRadius: 11,
                  border: "1px dashed var(--scry-border2)",
                  background: "transparent",
                  color: column.headingColor,
                }}
              >
                <Plus className="h-4 w-4" style={{ color: column.headingColor }} />
                {t("setup.addInstance", { product: column.productName })}
              </button>
            </div>
          );
        })}
      </div>

      {/* Read-only provenance note + at-least-one hint. */}
      <div className="mt-6 flex flex-col items-center gap-2 text-center">
        <p
          className="flex items-center justify-center gap-1.5"
          style={{
            fontSize: 12.5,
            color: "var(--scry-faint)",
            maxWidth: 620,
            lineHeight: 1.5,
          }}
        >
          <Info className="h-3.5 w-3.5 shrink-0" />
          {t("setup.atLeastOneRequired")}
        </p>
        <p
          style={{
            fontSize: 12,
            color: "var(--scry-muted3)",
            maxWidth: 620,
            lineHeight: 1.5,
          }}
        >
          {t("setup.connectReadOnlyNote")}
        </p>
      </div>
    </div>
  );
}

interface InstanceCardProps {
  inst: ImportInstance;
  urlPlaceholder: string;
  t: (key: string, values?: Record<string, unknown>) => string;
  onName: (name: string) => void;
  onRemove: () => void;
  onField: (field: "baseUrl" | "apiKey", value: string) => void;
  onVerifyBlur: () => void;
}

function InstanceCard({
  inst,
  urlPlaceholder,
  t,
  onName,
  onRemove,
  onField,
  onVerifyBlur,
}: InstanceCardProps) {
  const [editingName, setEditingName] = useState(false);
  const colors = instancePillColors(inst.kind);

  const named = inst.name.trim().length > 0;
  const nameDisplay = named ? inst.name : t("setup.unnamedInstance");

  return (
    <div
      data-slot="import-instance-card"
      className="flex flex-col gap-[11px]"
      style={{
        border: "1px solid var(--scry-border)",
        borderRadius: 14,
        background: "rgba(10, 17, 32, 0.5)",
        padding: "13px 14px 14px",
      }}
    >
      {/* Top row: status dot + name (+ rename) + remove. */}
      <div className="flex items-center gap-2">
        <span
          aria-hidden
          style={{
            width: 7,
            height: 7,
            borderRadius: "50%",
            flex: "none",
            background: statusDotColor(inst.status, colors.dot),
            boxShadow: `0 0 0 3px ${colors.bg}`,
          }}
        />
        {editingName ? (
          <Input
            autoFocus
            value={inst.name}
            placeholder={t("setup.instanceName")}
            onChange={(e) => onName(e.target.value)}
            onBlur={() => setEditingName(false)}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === "Escape") {
                e.currentTarget.blur();
              }
            }}
            className="h-7 flex-1 rounded-md text-[13.5px] font-semibold text-white"
            style={{
              border: "1px solid var(--scry-accent)",
              background: "var(--scry-chip)",
            }}
          />
        ) : (
          <span
            className="flex-1 truncate"
            title={nameDisplay}
            style={{
              fontSize: 13.5,
              fontWeight: 600,
              color: named ? "#eaf1ff" : "var(--scry-faint)",
            }}
          >
            {nameDisplay}
          </span>
        )}
        {!editingName ? (
          <button
            type="button"
            onClick={() => setEditingName(true)}
            title={t("setup.renameInstance")}
            aria-label={t("setup.renameInstance")}
            className="flex items-center justify-center transition-colors hover:text-[var(--scry-accent-text)]"
            style={{
              width: 24,
              height: 24,
              flex: "none",
              borderRadius: 7,
              border: "1px solid var(--scry-border2)",
              background: "transparent",
              color: "var(--scry-faint)",
            }}
          >
            <Pencil style={{ width: 13, height: 13 }} />
          </button>
        ) : null}
        <button
          type="button"
          onClick={onRemove}
          title={t("setup.removeInstance")}
          aria-label={t("setup.removeInstance")}
          className="flex items-center justify-center transition-colors hover:text-[#f87171]"
          style={{
            width: 26,
            height: 26,
            flex: "none",
            marginLeft: "auto",
            borderRadius: 7,
            border: "1px solid var(--scry-border2)",
            background: "transparent",
            color: "var(--scry-faint)",
          }}
        >
          <X style={{ width: 14, height: 14 }} />
        </button>
      </div>

      {/* URL field. */}
      <div>
        <label style={FIELD_LABEL_STYLE}>URL</label>
        <Input
          value={inst.baseUrl}
          spellCheck={false}
          autoComplete="off"
          placeholder={urlPlaceholder}
          onChange={(e) => onField("baseUrl", e.target.value)}
          onBlur={onVerifyBlur}
          className={MONO_INPUT_CLASS}
        />
      </div>

      {/* API Key field. */}
      <div>
        <label style={FIELD_LABEL_STYLE}>API Key</label>
        <Input
          type="password"
          value={inst.apiKey}
          spellCheck={false}
          autoComplete="off"
          placeholder={t("setup.apiKeyHelpHint")}
          onChange={(e) => onField("apiKey", e.target.value)}
          onBlur={onVerifyBlur}
          className={MONO_INPUT_CLASS}
        />
      </div>

      {/* Status row. */}
      <StatusRow inst={inst} t={t} />
    </div>
  );
}

function StatusRow({
  inst,
  t,
}: {
  inst: ImportInstance;
  t: (key: string, values?: Record<string, unknown>) => string;
}) {
  return (
    <div
      className="flex items-center gap-1.5"
      style={{ minHeight: 30, fontSize: 13 }}
    >
      {inst.status === "connected" ? (
        <span
          className="flex items-center gap-1.5"
          style={{ color: "#4ade80", fontWeight: 600 }}
        >
          <CircleCheckBig style={{ width: 15, height: 15 }} />
          {t("setup.connected")}
          {inst.version ? (
            <span style={{ color: "var(--scry-faint)", fontWeight: 500 }}>
              {inst.version}
            </span>
          ) : null}
        </span>
      ) : inst.status === "testing" ? (
        <span
          className="flex items-center gap-1.5"
          style={{ color: "var(--scry-accent-text)" }}
        >
          <Loader2 className="animate-spin" style={{ width: 15, height: 15 }} />
          {t("setup.testing")}
        </span>
      ) : inst.status === "error" ? (
        <span
          className="flex items-center gap-1.5"
          style={{ color: "#f87171" }}
          title={inst.error ?? undefined}
        >
          <CircleAlert style={{ width: 15, height: 15 }} />
          {t("setup.couldntConnect")}
        </span>
      ) : (
        <span
          className="flex items-center gap-1.5"
          style={{ color: "var(--scry-faint)" }}
        >
          <KeyRound style={{ width: 14, height: 14 }} />
          {t("setup.enterUrlAndKey")}
        </span>
      )}
    </div>
  );
}

function statusDotColor(
  status: ImportInstance["status"],
  idleColor: string,
): string {
  switch (status) {
    case "connected":
      return "#4ade80";
    case "testing":
      return "var(--scry-accent)";
    case "error":
      return "#f87171";
    default:
      return idleColor;
  }
}

export default SetupImportConnectView;
