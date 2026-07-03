import CodeEditor, {
  type CodeEditorDiagnostic,
  type CodeEditorProps,
} from "@/components/common/code-editor";

export type RegoEditorDiagnostic = CodeEditorDiagnostic;

export type RegoEditorProps = Omit<CodeEditorProps, "language">;

export default function RegoEditor(props: RegoEditorProps) {
  return <CodeEditor {...props} language="rego" />;
}
