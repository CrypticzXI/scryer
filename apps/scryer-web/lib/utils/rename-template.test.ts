import test from "node:test";
import assert from "node:assert/strict";

import {
  applyRenameTemplatePreview,
  splitRenameTemplateSegments,
  validateRenameTemplateSyntax,
} from "./rename-template.ts";

const VALID_TOKENS = new Set(["title", "season_order", "edition", "ext"]);
const SAMPLE_VALUES = {
  title: "The Dark Knight",
  edition: "IMAX",
  ext: "mkv",
};

test("validateRenameTemplateSyntax accepts truncate filters", () => {
  assert.equal(
    validateRenameTemplateSyntax("{title|truncate:8|space:_}.{ext}", VALID_TOKENS),
    null,
  );
});

test("validateRenameTemplateSyntax rejects invalid truncate filters", () => {
  assert.deepEqual(validateRenameTemplateSyntax("{title|truncate:0}", VALID_TOKENS), {
    kind: "invalidFilter",
    filter: "truncate:0",
  });
  assert.deepEqual(validateRenameTemplateSyntax("{title|truncate:abc}", VALID_TOKENS), {
    kind: "invalidFilter",
    filter: "truncate:abc",
  });
});

test("validateRenameTemplateSyntax accepts literal brace escapes", () => {
  assert.equal(
    validateRenameTemplateSyntax("{{edition-{edition}}}", VALID_TOKENS),
    null,
  );
});

test("validateRenameTemplateSyntax rejects unmatched single braces", () => {
  assert.deepEqual(validateRenameTemplateSyntax("prefix {", VALID_TOKENS), {
    kind: "unmatchedOpen",
  });
  assert.deepEqual(validateRenameTemplateSyntax("prefix }", VALID_TOKENS), {
    kind: "unmatchedClose",
  });
});

test("applyRenameTemplatePreview applies truncate before later filters", () => {
  assert.equal(
    applyRenameTemplatePreview(
      "{title|truncate:8|space:_}.{ext}",
      VALID_TOKENS,
      SAMPLE_VALUES,
    ),
    "The_Dark.mkv",
  );
});

test("applyRenameTemplatePreview renders literal brace escapes", () => {
  assert.equal(
    applyRenameTemplatePreview(
      "{{edition-{edition}}}",
      VALID_TOKENS,
      SAMPLE_VALUES,
    ),
    "{edition-IMAX}",
  );
});

test("applyRenameTemplatePreview renders missing sample values as empty strings", () => {
  assert.equal(
    applyRenameTemplatePreview("{title} - {season_order}.{ext}", VALID_TOKENS, SAMPLE_VALUES),
    "The Dark Knight - .mkv",
  );
});

test("splitRenameTemplateSegments highlights filtered token specs", () => {
  assert.deepEqual(
    splitRenameTemplateSegments("{title|truncate:8|space:_}.{ext}", VALID_TOKENS),
    [
      { text: "{title|truncate:8|space:_}", isToken: true },
      { text: ".", isToken: false },
      { text: "{ext}", isToken: true },
    ],
  );
});
