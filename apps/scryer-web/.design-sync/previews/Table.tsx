// Authored preview for the Scryer <Table> (wide → cardMode column). A movies list.
import { Table, TableHeader, TableBody, TableRow, TableHead, TableCell } from "scryer-web";

const rows = [
  { title: "Dune: Part Two", year: 2024, quality: "2160p", status: "Downloaded" },
  { title: "Oppenheimer", year: 2023, quality: "1080p", status: "Downloaded" },
  { title: "The Batman", year: 2022, quality: "1080p", status: "Missing" },
  { title: "Blade Runner 2049", year: 2017, quality: "2160p", status: "Monitored" },
];

export function Movies() {
  return (
    <div className="p-6">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Title</TableHead>
            <TableHead>Year</TableHead>
            <TableHead>Quality</TableHead>
            <TableHead>Status</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {rows.map((r) => (
            <TableRow key={r.title}>
              <TableCell className="font-medium text-foreground">{r.title}</TableCell>
              <TableCell className="text-muted-foreground">{r.year}</TableCell>
              <TableCell className="text-muted-foreground">{r.quality}</TableCell>
              <TableCell className="text-muted-foreground">{r.status}</TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </div>
  );
}
