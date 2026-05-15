import { Info } from "lucide-react";
import {
  HoverCard,
  HoverCardContent,
  HoverCardTrigger,
} from "@/components/ui/hover-card";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { useIsMobile } from "@/lib/hooks/use-mobile";

type InfoHelpProps = {
  text: string;
  ariaLabel: string;
};

export function InfoHelp({ text, ariaLabel }: InfoHelpProps) {
  const isMobile = useIsMobile();
  const trigger = (
    <button
      type="button"
      className="rounded p-0.5 text-muted-foreground transition hover:text-card-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400/70"
      aria-label={ariaLabel}
    >
      <Info className="h-3.5 w-3.5" />
    </button>
  );

  if (isMobile) {
    return (
      <Popover>
        <PopoverTrigger asChild>{trigger}</PopoverTrigger>
        <PopoverContent align="start">
          <p className="max-w-[28rem] whitespace-normal break-words">{text}</p>
        </PopoverContent>
      </Popover>
    );
  }

  return (
    <HoverCard openDelay={150} closeDelay={75}>
      <HoverCardTrigger asChild>{trigger}</HoverCardTrigger>
      <HoverCardContent>
        <p className="max-w-[28rem] whitespace-normal break-words">{text}</p>
      </HoverCardContent>
    </HoverCard>
  );
}
