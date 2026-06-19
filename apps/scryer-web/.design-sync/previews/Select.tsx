// Authored preview for the Scryer <Select> — rendered open so the dropdown items show.
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem, SelectGroup, SelectLabel } from "scryer-web";

export function Open() {
  return (
    <div className="flex justify-center pt-4">
      <Select defaultValue="1080p" defaultOpen>
        <SelectTrigger className="w-56">
          <SelectValue placeholder="Quality profile" />
        </SelectTrigger>
        <SelectContent>
          <SelectGroup>
            <SelectLabel>Quality profile</SelectLabel>
            <SelectItem value="2160p">2160p (4K)</SelectItem>
            <SelectItem value="1080p">1080p</SelectItem>
            <SelectItem value="720p">720p</SelectItem>
            <SelectItem value="sd">SD</SelectItem>
          </SelectGroup>
        </SelectContent>
      </Select>
    </div>
  );
}
