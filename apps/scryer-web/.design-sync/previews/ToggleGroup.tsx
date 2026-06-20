// Authored preview for the Scryer <ToggleGroup> (single-select; on = primary).
import { ToggleGroup, ToggleGroupItem } from "scryer-web";

const Grid = () => (
  <svg viewBox="0 0 24 24" className="size-4" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
    <rect x="3" y="3" width="7" height="7" rx="1" /><rect x="14" y="3" width="7" height="7" rx="1" /><rect x="3" y="14" width="7" height="7" rx="1" /><rect x="14" y="14" width="7" height="7" rx="1" />
  </svg>
);
const List = () => (
  <svg viewBox="0 0 24 24" className="size-4" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
    <path d="M8 6h13M8 12h13M8 18h13M3 6h.01M3 12h.01M3 18h.01" />
  </svg>
);

export function ViewMode() {
  return (
    <div className="p-6">
      <ToggleGroup type="single" defaultValue="grid">
        <ToggleGroupItem value="grid"><Grid />Grid</ToggleGroupItem>
        <ToggleGroupItem value="list"><List />List</ToggleGroupItem>
        <ToggleGroupItem value="detail">Detail</ToggleGroupItem>
      </ToggleGroup>
    </div>
  );
}

export function Outline() {
  return (
    <div className="p-6">
      <ToggleGroup type="single" defaultValue="all" variant="outline">
        <ToggleGroupItem value="all">All</ToggleGroupItem>
        <ToggleGroupItem value="monitored">Monitored</ToggleGroupItem>
        <ToggleGroupItem value="missing">Missing</ToggleGroupItem>
      </ToggleGroup>
    </div>
  );
}
