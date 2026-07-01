
import {
  CircleCheckIcon,
  InfoIcon,
  Loader2Icon,
  OctagonXIcon,
  TriangleAlertIcon,
} from "lucide-react"
import { useTheme } from "next-themes"
import { Toaster as Sonner, toast, type ToasterProps } from "sonner"
import { isDarkTheme } from "@/lib/theme"

const Toaster = ({ style, toastOptions, ...props }: ToasterProps) => {
  const { resolvedTheme } = useTheme()
  const isDark = isDarkTheme(resolvedTheme)

  const themeStyle = (
    isDark
      ? {
          "--normal-bg": "var(--popover)",
          "--normal-text": "var(--popover-foreground)",
          "--normal-border": "var(--border-color)",
          "--normal-bg-hover": "var(--card)",
          "--normal-border-hover": "var(--border-color)",
          "--success-bg": "rgb(2, 23, 18)",
          "--success-border": "rgb(16, 185, 129)",
          "--success-text": "rgb(209, 250, 229)",
          "--error-bg": "var(--scry-danger-bg)",
          "--error-border": "var(--scry-danger-border)",
          "--error-text": "var(--scry-danger-text)",
          "--warning-bg": "var(--scry-warning-bg)",
          "--warning-border": "var(--scry-warning-border)",
          "--warning-text": "var(--scry-warning-text)",
          "--info-bg": "rgb(7, 22, 42)",
          "--info-border": "rgb(96, 165, 250)",
          "--info-text": "rgb(219, 234, 254)",
          "--border-radius": "var(--radius)",
        }
      : {
          "--normal-bg": "var(--popover)",
          "--normal-text": "var(--popover-foreground)",
          "--normal-border": "var(--border-color)",
          "--normal-bg-hover": "var(--card)",
          "--normal-border-hover": "var(--border-color)",
          "--success-bg": "rgb(240, 253, 244)",
          "--success-border": "rgb(16, 185, 129)",
          "--success-text": "rgb(5, 46, 22)",
          "--error-bg": "var(--scry-danger-bg)",
          "--error-border": "var(--scry-danger-border)",
          "--error-text": "var(--scry-danger-text)",
          "--warning-bg": "var(--scry-warning-bg)",
          "--warning-border": "var(--scry-warning-border)",
          "--warning-text": "var(--scry-warning-text)",
          "--info-bg": "rgb(239, 246, 255)",
          "--info-border": "rgb(96, 165, 250)",
          "--info-text": "rgb(30, 58, 138)",
          "--border-radius": "var(--radius)",
        }
  ) as React.CSSProperties

  return (
    <Sonner
      theme={isDark ? "dark" : "light"}
      richColors
      className="toaster group"
      toastOptions={{
        className: "bg-background shadow-sm shadow-black/35",
        classNames: {
          toast: "rounded-lg border border-border/30",
          success: isDark ? "border-emerald-500 bg-emerald-950" : "border-emerald-500 bg-emerald-50",
          error: "border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)]",
          warning: "border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)]",
          info: "border-sky-400/55",
        },
        ...toastOptions,
      }}
      icons={{
        success: <CircleCheckIcon className="size-4" />,
        info: <InfoIcon className="size-4" />,
        warning: <TriangleAlertIcon className="size-4" />,
        error: <OctagonXIcon className="size-4" />,
        loading: <Loader2Icon className="size-4 animate-spin" />,
      }}
      style={{ ...themeStyle, ...style } as React.CSSProperties}
      {...props}
    />
  )
}

export { Toaster, toast }
