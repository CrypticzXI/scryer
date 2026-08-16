import assert from "node:assert/strict";
import test from "node:test";

import { parseStoredCalendarViewMode } from "./calendar-view-mode.ts";

test("calendar view mode accepts only supported FullCalendar views", () => {
  assert.equal(parseStoredCalendarViewMode("dayGridMonth"), "dayGridMonth");
  assert.equal(parseStoredCalendarViewMode("dayGridWeek"), "dayGridWeek");
  assert.equal(parseStoredCalendarViewMode("dayGridDay"), null);
  assert.equal(parseStoredCalendarViewMode(null), null);
});
