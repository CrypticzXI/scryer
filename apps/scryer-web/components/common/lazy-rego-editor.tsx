import {
  LazyCodeEditor,
  type CodeEditorDiagnostic,
  type CodeEditorProps,
} from "@/components/common/lazy-code-editor";

export type RegoEditorDiagnostic = CodeEditorDiagnostic;

export type LazyRegoEditorProps = Omit<CodeEditorProps, "language">;

export function LazyRegoEditor(props: LazyRegoEditorProps) {
  return <LazyCodeEditor {...props} language="rego" />;
}
