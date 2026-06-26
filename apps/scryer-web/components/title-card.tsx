import * as React from "react";
import { Clock, Eye, EyeOff, Plus, Send } from "lucide-react";
import type { Facet } from "@/lib/types/titles";
import { TitlePosterSlot } from "@/components/title-poster-slot";
import { useTranslate } from "@/lib/context/translate-context";
import { cn } from "@/lib/utils";

const FACET_BADGE_TEXT_CLASS: Record<Facet, string> = {
  movie: "text-[#a9b3ff]",
  series: "text-[#7cc4ff]",
  anime: "text-[#d9a9ff]",
};

const FACET_LABEL_KEY: Record<Facet, string> = {
  movie: "search.facetMovie",
  series: "search.facetSeries",
  anime: "search.facetAnime",
};

const ACTION_BUTTON_CLASS =
  "pointer-events-auto flex h-14 w-14 items-center justify-center rounded-[16px] text-white shadow-lg transition hover:brightness-110 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white/80 disabled:cursor-default";

const ACCENT_ACTION_STYLE: React.CSSProperties = {
  backgroundImage: "var(--scry-accent-grad)",
  boxShadow: "0 12px 28px rgba(var(--scry-accent-rgb), 0.45)",
};

const REQUESTED_ACTION_STYLE: React.CSSProperties = {
  backgroundImage: "linear-gradient(135deg, #e6b347, #c2851a)",
  boxShadow: "0 12px 28px rgba(206, 150, 40, 0.4)",
};

export type TitleCardProps = {
  /** Display title, e.g. "Oppenheimer". */
  title: string;
  /** Release year (or any short subtitle), shown under the title. */
  year?: number | string | null;
  /** Facet drives the top-left badge label + color. */
  facet?: Facet | null;
  /** Override the badge text (already localized). Defaults to the facet label. */
  facetLabel?: string | null;
  posterUrl?: string | null;
  posterSourceUrl?: string | null;
  metadataFetchedAt?: string | null;
  createdAt?: string | null;
  /** Operator with library access can add it → centered "+" action. */
  addable?: boolean;
  /** Member without direct access can request it → centered paper-airplane. */
  requestable?: boolean;
  /** Already requested / pending → amber clock; takes precedence over add/request. */
  requested?: boolean;
  /** Library monitored state. When non-null, shows an eye / eye-off indicator. */
  monitored?: boolean | null;
  /** Click the card body (opens overview/detail). When omitted, the body is inert. */
  onOpen?: () => void;
  onAdd?: () => void;
  onRequest?: () => void;
  selected?: boolean;
  className?: string;
};

/**
 * The single, consistent interactive poster used everywhere a movie, series, or
 * anime is surfaced — facet overviews and every discovery surface. A poster
 * under a frosted-glass veil with its title at the base; the action lives in the
 * center: "+" to add (operators), a paper airplane to request (members), both
 * side by side when a person can do either, an amber clock once requested, and
 * no action when it's browse-only.
 */
export function TitleCard({
  title,
  year,
  facet,
  facetLabel,
  posterUrl,
  posterSourceUrl,
  metadataFetchedAt,
  createdAt,
  addable = false,
  requestable = false,
  requested = false,
  monitored,
  onOpen,
  onAdd,
  onRequest,
  selected = false,
  className,
}: TitleCardProps) {
  const t = useTranslate();
  const badgeLabel =
    facetLabel ?? (facet ? t(FACET_LABEL_KEY[facet]) : null);
  const badgeColorClass = facet
    ? FACET_BADGE_TEXT_CLASS[facet]
    : "text-[var(--scry-muted2)]";
  const hasYear = year != null && `${year}`.trim() !== "";

  return (
    <div
      className={cn(
        "group relative aspect-[2/3] w-full overflow-hidden rounded-[16px] border border-[var(--scry-border2)] bg-[var(--scry-card2)]",
        selected && "ring-2 ring-[var(--scry-accent-ring)]",
        className,
      )}
    >
      {/* Frosted poster backdrop */}
      <div className="absolute inset-0">
        <TitlePosterSlot
          src={posterUrl}
          sourceSrc={posterSourceUrl}
          metadataFetchedAt={metadataFetchedAt}
          createdAt={createdAt}
          alt={title}
          emptyLabel=""
          className="h-full w-full scale-110 object-cover blur-xl brightness-[0.55] saturate-[0.92]"
          placeholderClassName="flex h-full w-full items-center justify-center bg-[var(--scry-card2)]"
          loading="lazy"
          decoding="async"
        />
        <div
          aria-hidden="true"
          className="absolute inset-0 bg-gradient-to-b from-black/30 via-black/15 to-black/70"
        />
      </div>

      {/* Card body click target (paints under the action buttons) */}
      {onOpen ? (
        <button
          type="button"
          onClick={onOpen}
          aria-label={title}
          className="absolute inset-0 z-0 cursor-pointer rounded-[16px] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-accent-ring)]"
        />
      ) : null}

      {/* Top-left: facet badge + optional monitored indicator */}
      <div className="pointer-events-none absolute left-2.5 top-2.5 z-10 flex items-center gap-1.5">
        {badgeLabel ? (
          <span
            className={cn(
              "rounded-md border border-white/10 bg-[rgba(4,6,12,0.7)] px-2 py-0.5 text-[10px] font-bold uppercase tracking-[0.06em] backdrop-blur-[4px]",
              badgeColorClass,
            )}
          >
            {badgeLabel}
          </span>
        ) : null}
        {monitored != null ? (
          <span className="flex h-[25px] w-[25px] items-center justify-center rounded-[7px] border border-white/10 bg-[rgba(4,6,12,0.7)] backdrop-blur-[4px]">
            {monitored ? (
              <Eye className="h-3.5 w-3.5 text-emerald-400" />
            ) : (
              <EyeOff className="h-3.5 w-3.5 text-[var(--scry-faint2)]" />
            )}
          </span>
        ) : null}
      </div>

      {/* Centered action(s) */}
      <div className="pointer-events-none absolute inset-0 z-10 flex items-center justify-center gap-2.5">
        {requested ? (
          <span
            className={cn(ACTION_BUTTON_CLASS, "cursor-default")}
            style={REQUESTED_ACTION_STYLE}
            title={t("discovery.requested")}
            aria-label={t("discovery.requested")}
            role="img"
          >
            <Clock className="h-6 w-6" />
          </span>
        ) : (
          <>
            {addable ? (
              <button
                type="button"
                onClick={onAdd}
                className={ACTION_BUTTON_CLASS}
                style={ACCENT_ACTION_STYLE}
                title={t("discovery.add")}
                aria-label={`${t("discovery.add")}: ${title}`}
              >
                <Plus className="h-6 w-6" />
              </button>
            ) : null}
            {requestable ? (
              <button
                type="button"
                onClick={onRequest}
                className={ACTION_BUTTON_CLASS}
                style={ACCENT_ACTION_STYLE}
                title={t("discovery.request")}
                aria-label={`${t("discovery.request")}: ${title}`}
              >
                <Send className="h-5 w-5" />
              </button>
            ) : null}
          </>
        )}
      </div>

      {/* Title + year at the base */}
      <div className="pointer-events-none absolute inset-x-0 bottom-0 z-10 px-3 pb-3.5 pt-12 text-center">
        <p
          className="line-clamp-2 text-[17px] font-bold leading-tight text-white drop-shadow-[0_1px_3px_rgba(0,0,0,0.65)]"
          style={{
            fontFamily:
              "var(--font-space-grotesk), var(--font-inter), ui-sans-serif, system-ui, -apple-system, sans-serif",
          }}
        >
          {title}
        </p>
        {hasYear ? (
          <p className="mt-0.5 text-[12.5px] font-medium text-white/55">
            {year}
          </p>
        ) : null}
      </div>
    </div>
  );
}
