import type { SyntheticEvent } from "react";

import { IconButton } from "@/components/ui/icon-button";
import { useTranslate } from "@/lib/context/translate-context";
import { cn } from "@/lib/utils";

export type MediaServerPlaybackLink = {
  connectionId: string;
  displayName: string;
  provider: "JELLYFIN" | "PLEX" | "EMBY";
  href: string;
};

const providerIconSrc: Record<MediaServerPlaybackLink["provider"], string> = {
  JELLYFIN: "/auth-providers/jellyfin.svg",
  PLEX: "/auth-providers/plex.svg",
  EMBY: "/auth-providers/emby.svg",
};

const providerLabel: Record<MediaServerPlaybackLink["provider"], string> = {
  JELLYFIN: "Jellyfin",
  PLEX: "Plex",
  EMBY: "Emby",
};

/** Direct provider links for a title or episode that passed playback authorization. */
export function WatchInMediaServerMenu({
  links,
  className,
  compact = false,
}: {
  links?: MediaServerPlaybackLink[] | null;
  className?: string;
  compact?: boolean;
}) {
  const t = useTranslate();
  if (!links || links.length === 0) return null;

  const stopParentNavigation = (event: SyntheticEvent) => {
    event.stopPropagation();
  };

  return (
    <div
      role="group"
      aria-label={t("label.watchIn")}
      className={cn("flex flex-wrap items-center gap-1.5", className)}
    >
      {links.map((link) => {
        const provider = providerLabel[link.provider];
        const label = `${t("label.watchIn")} ${provider} — ${link.displayName}`;

        return (
          <IconButton
            key={link.connectionId}
            asChild
            label={label}
            tooltipSide="top"
            appearance={compact ? "ghost" : "boxed"}
            className={cn(
              "shrink-0 rounded-[9px] [&_img]:transition-transform [&:hover_img]:scale-105",
              compact ? "h-7 w-7" : "h-9 w-9",
            )}
          >
            <a
              href={link.href}
              target="_blank"
              rel="noopener noreferrer"
              onClick={stopParentNavigation}
              onPointerDown={stopParentNavigation}
            >
              <img
                src={providerIconSrc[link.provider]}
                alt=""
                aria-hidden="true"
                className={cn(
                  "object-contain",
                  compact ? "h-4 w-4" : "h-[19px] w-[19px]",
                )}
              />
            </a>
          </IconButton>
        );
      })}
    </div>
  );
}
