// Authored preview for the Scryer <HoverCard> — rendered open so the floating card shows.
import { HoverCard, HoverCardTrigger, HoverCardContent, Button } from "scryer-web";

export function Default() {
  return (
    <div className="flex justify-center pt-6">
      <HoverCard defaultOpen>
        <HoverCardTrigger asChild>
          <Button variant="link">Dune: Part Two</Button>
        </HoverCardTrigger>
        <HoverCardContent>
          <div className="flex flex-col gap-1">
            <div className="text-sm font-semibold text-foreground">Dune: Part Two (2024)</div>
            <div className="text-xs text-muted-foreground">2160p · BluRay · 18.4 GB · monitored</div>
            <p className="text-xs text-muted-foreground">
              Paul Atreides unites with the Fremen to wage war against House Harkonnen.
            </p>
          </div>
        </HoverCardContent>
      </HoverCard>
    </div>
  );
}
