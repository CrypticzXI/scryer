import test from "node:test";
import assert from "node:assert/strict";

import {
  applyRenameTemplatePreview,
  splitRenameTemplateSegments,
  validateRenameTemplateSyntax,
} from "./rename-template.ts";

const VALID_TOKENS = new Set(["title", "ext"]);
const SAMPLE_VALUES = {
  title: "The Dark Knight",
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
