import { lazy, Suspense } from "react";

const RegoEditor = lazy(() => import("./rego-editor"));

type LazyRegoEditorProps = {
  id?: string;
  value: string;
  onChange: (value: string) => void;
  readOnly?: boolean;
  height?: string;
  minLines?: number;
  maxLines?: number;
};

function TextareaFallback({
  id,
  value,
  onChange,
  readOnly,
  height = "320px",
  minLines,
  maxLines,
}: LazyRegoEditorProps) {
  const lineCount = value.split("\n").length;
  const lineHeightPx = 20;
  const autoHeight =
    minLines && maxLines
      ? `${Math.min(Math.max(lineCount, minLines), maxLines) * lineHeightPx}px`
      : height;

  return (
    <textarea
      id={id}
      value={value}
      onChange={(e) => onChange(e.target.value)}
      readOnly={readOnly}
      className="w-full rounded-md border border-border bg-background p-3 font-[var(--font-code)] text-sm text-foreground"
      style={{ height: autoHeight, minHeight: "120px", resize: "vertical" }}
    />
  );
}

export function LazyRegoEditor(props: LazyRegoEditorProps) {
  return (
    <Suspense fallback={<TextareaFallback {...props} />}>
      <RegoEditor {...props} />
    </Suspense>
  );
}
