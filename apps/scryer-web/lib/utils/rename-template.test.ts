import test from "node:test";
import assert from "node:assert/strict";

import {
  applyRenameTemplatePreview,
  splitRenameTemplateSegments,
  validateFolderTemplateSyntax,
  validateRenameTemplateSyntax,
} from "./rename-template.ts";

const VALID_TOKENS = new Set(["title", "season_order", "edition", "ext"]);
const VALID_FOLDER_TOKENS = new Set(["title", "season"]);
const SAMPLE_VALUES = {
  title: "The Grey Harbor",
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

test("validateFolderTemplateSyntax accepts deterministic season padding", () => {
  for (const template of ["Season {season}", "Season {season:0}", "Season {season:2}"]) {
    assert.equal(validateFolderTemplateSyntax(template, VALID_FOLDER_TOKENS, "season"), null);
  }
  assert.equal(
    applyRenameTemplatePreview("Season {season:2}", VALID_FOLDER_TOKENS, { season: "3" }),
    "Season 03",
  );
});

test("validateFolderTemplateSyntax rejects malformed or excessive padding", () => {
  for (const [template, padding] of [
    ["Season {season:}", ""],
    ["Season {season:abc}", "abc"],
    ["Season {season:2x}", "2x"],
    ["Season {season:241}", "241"],
    ["Season {season:999999999999999999999999999999999999999}", "999999999999999999999999999999999999999"],
  ]) {
    assert.deepEqual(validateFolderTemplateSyntax(template, VALID_FOLDER_TOKENS, "season"), {
      kind: "invalidPadding",
      padding,
    });
  }
});

test("validateFolderTemplateSyntax rejects illegal literal characters", () => {
  for (const character of ["<", ">", ":", "\"", "/", "\\", "|", "?", "*", "\n"]) {
    assert.deepEqual(
      validateFolderTemplateSyntax(`Season${character} {season}`, VALID_FOLDER_TOKENS, "season"),
      { kind: "illegalCharacter", character },
    );
  }
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
    "The_Grey.mkv",
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
    "The Grey Harbor - .mkv",
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
