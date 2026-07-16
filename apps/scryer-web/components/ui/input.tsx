import * as React from "react"

import { cn } from "@/lib/utils"

export const integerInputProps = {
  type: "text",
  inputMode: "numeric",
  pattern: "[0-9]*",
} satisfies Pick<
  React.ComponentProps<"input">,
  "type" | "inputMode" | "pattern"
>

export const signedIntegerInputProps = {
  type: "number",
  inputMode: "numeric",
  step: 1,
} satisfies Pick<
  React.ComponentProps<"input">,
  "type" | "inputMode" | "step"
>

export function sanitizeDigits(raw: string): string {
  return raw.replace(/\D+/g, "")
}

type InputProps = React.ComponentProps<"input"> & {
  "data-1p-ignore"?: string
  "data-lpignore"?: string
  "data-form-type"?: string
}

const credentialAutocompleteValues = new Set([
  "current-password",
  "new-password",
  "one-time-code",
  "username",
])

function shouldSuppressPasswordManager(
  type: React.HTMLInputTypeAttribute | undefined,
  autoComplete: string | undefined,
): boolean {
  if (type === "password") {
    return false
  }

  const normalizedAutoComplete = autoComplete?.toLowerCase()
  return !(
    normalizedAutoComplete &&
    credentialAutocompleteValues.has(normalizedAutoComplete)
  )
}

function Input({
  className,
  type,
  autoComplete,
  "data-1p-ignore": onePasswordIgnore,
  "data-lpignore": lastPassIgnore,
  "data-form-type": formType,
  ...props
}: InputProps) {
  const suppressPasswordManager = shouldSuppressPasswordManager(
    type,
    autoComplete,
  )

  return (
    <input
      type={type}
      data-slot="input"
      autoComplete={autoComplete ?? (suppressPasswordManager ? "off" : undefined)}
      data-1p-ignore={onePasswordIgnore ?? (suppressPasswordManager ? "true" : undefined)}
      data-lpignore={lastPassIgnore ?? (suppressPasswordManager ? "true" : undefined)}
      data-form-type={formType ?? (suppressPasswordManager ? "other" : undefined)}
      className={cn(
        "file:text-foreground placeholder:text-muted-foreground selection:bg-primary selection:text-primary-foreground bg-field text-foreground border-input h-9 w-full min-w-0 rounded-md border px-3 py-1 text-base shadow-xs transition-[color,box-shadow] outline-none file:inline-flex file:h-7 file:border-0 file:bg-transparent file:text-sm file:font-medium disabled:pointer-events-none disabled:cursor-not-allowed disabled:opacity-50 md:text-sm",
        "focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-[3px]",
        "aria-invalid:ring-destructive/20 aria-invalid:ring-destructive/40 aria-invalid:border-destructive",
        className
      )}
      {...props}
    />
  )
}

export { Input }
