import { Loader2 } from "lucide-react";
import { Input, integerInputProps, sanitizeDigits } from "@/components/ui/input";
import { cn } from "@/lib/utils";

export function sanitizeTotpCode(value: string): string {
  return sanitizeDigits(value).slice(0, 6);
}

type TotpCodeFormProps = {
  id?: string;
  code: string;
  title: string;
  description: string;
  submitLabel: string;
  busyLabel?: string;
  cancelLabel?: string;
  inputId?: string;
  submitId?: string;
  cancelId?: string;
  busy?: boolean;
  disabled?: boolean;
  autoFocus?: boolean;
  className?: string;
  onCodeChange: (code: string) => void;
  onSubmit: () => void | Promise<void>;
  onCancel?: () => void;
};

export function TotpCodeForm({
  id,
  code,
  title,
  description,
  submitLabel,
  busyLabel,
  cancelLabel,
  inputId,
  submitId,
  cancelId,
  busy = false,
  disabled = false,
  autoFocus = true,
  className,
  onCodeChange,
  onSubmit,
  onCancel,
}: TotpCodeFormProps) {
  const submitDisabled = disabled || busy || code.length !== 6;

  return (
    <form
      id={id}
      onSubmit={(event) => {
        event.preventDefault();
        if (!submitDisabled) {
          void onSubmit();
        }
      }}
      className={cn("space-y-4", className)}
    >
      <div className="space-y-1 text-center">
        <h2 className="text-base font-semibold">{title}</h2>
        <p className="text-sm text-muted-foreground">{description}</p>
      </div>
      <Input
        {...integerInputProps}
        id={inputId}
        autoComplete="one-time-code"
        autoFocus={autoFocus}
        maxLength={6}
        value={code}
        onChange={(event) => onCodeChange(sanitizeTotpCode(event.target.value))}
        placeholder={title}
      />
      <button
        id={submitId}
        type="submit"
        disabled={submitDisabled}
        className="flex w-full items-center justify-center gap-2 rounded-md bg-[var(--scry-success-solid)] px-4 py-2 text-sm font-medium text-[var(--scry-success-on-solid)] hover:bg-[var(--scry-success-solid-hover)] disabled:opacity-50"
      >
        {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
        {busy && busyLabel ? busyLabel : submitLabel}
      </button>
      {onCancel && cancelLabel ? (
        <button
          id={cancelId}
          type="button"
          onClick={onCancel}
          disabled={disabled || busy}
          className="w-full rounded-md border border-border bg-background px-4 py-2 text-sm font-medium text-foreground hover:bg-muted disabled:opacity-50"
        >
          {cancelLabel}
        </button>
      ) : null}
    </form>
  );
}
