import type { CSSProperties } from "react";

import type { ImportInstanceKind } from "@/lib/hooks/use-external-import-setup";

type PillKind = ImportInstanceKind | "manual";

interface KindColors {
  dot: string;
  bg: string;
  border: string;
  text: string;
}

const KIND_COLORS: Record<PillKind, KindColors> = {
  sonarr: {
    dot: "var(--scry-facet-series)",
    bg: "var(--scry-facet-series-bg)",
    border: "var(--scry-facet-series-border)",
    text: "var(--scry-facet-series-text)",
  },
  radarr: {
    dot: "var(--scry-facet-movie)",
    bg: "var(--scry-facet-movie-bg)",
    border: "var(--scry-facet-movie-border)",
    text: "var(--scry-facet-movie-text)",
  },
  prowlarr: {
    dot: "var(--scry-facet-anime)",
    bg: "var(--scry-facet-anime-bg)",
    border: "var(--scry-facet-anime-border)",
    text: "var(--scry-facet-anime-text)",
  },
  manual: {
    dot: "var(--scry-faint)",
    bg: "var(--scry-chip)",
    border: "var(--scry-border2)",
    text: "var(--scry-muted2)",
  },
};

// Product logos live in public/ (served under import.meta.env.BASE_URL, which is
// "./" in production builds). Sonarr/Radarr sit at the root; Prowlarr under
// media-sites/.
const PRODUCT_LOGOS: Partial<Record<PillKind, string>> = {
  sonarr: `${import.meta.env.BASE_URL}sonarr.svg`,
  radarr: `${import.meta.env.BASE_URL}radarr.svg`,
  prowlarr: `${import.meta.env.BASE_URL}media-sites/prowlarr.svg`,
};

export function productLogoUrl(kind: PillKind): string | null {
  return PRODUCT_LOGOS[kind] ?? null;
}

interface ImportInstancePillProps {
  kind: PillKind;
  label: string;
  title?: string;
  size?: "sm" | "md";
  showDot?: boolean;
  className?: string;
  style?: CSSProperties;
}

/**
 * Small instance-identity pill (colored dot + short name), keyed by source kind.
 * Used in the Connect status, mapping board root chips, the assign sheet/remap
 * dialog source rows, and the Summary instance list.
 */
export function ImportInstancePill({
  kind,
  label,
  title,
  size = "md",
  showDot = true,
  className,
  style,
}: ImportInstancePillProps) {
  const colors = KIND_COLORS[kind];
  const small = size === "sm";
  const logoUrl = productLogoUrl(kind);
  return (
    <span
      title={title ?? label}
      className={className}
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: small ? 5 : 6,
        padding: small ? "3px 6px" : "3px 7px",
        borderRadius: 7,
        background: colors.bg,
        border: `1px solid ${colors.border}`,
        color: colors.text,
        fontSize: small ? 10 : 10.5,
        fontWeight: 600,
        whiteSpace: "nowrap",
        flex: "none",
        ...style,
      }}
    >
      {logoUrl ? (
        <img
          src={logoUrl}
          alt=""
          aria-hidden
          style={{
            width: small ? 14 : 16,
            height: small ? 14 : 16,
            objectFit: "contain",
            flex: "none",
          }}
        />
      ) : showDot ? (
        <span
          aria-hidden
          style={{
            width: 6,
            height: 6,
            borderRadius: "50%",
            background: colors.dot,
            flex: "none",
          }}
        />
      ) : null}
      {label}
    </span>
  );
}

export function instancePillColors(kind: PillKind): KindColors {
  return KIND_COLORS[kind];
}
