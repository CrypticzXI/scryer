import assert from "node:assert/strict";
import test from "node:test";
import { LatestWinsSaveQueue } from "./latest-wins-save-queue.ts";

test("LatestWinsSaveQueue saves the newest queued value after the active save", async () => {
  const queue = new LatestWinsSaveQueue<string>();
  const savedValues: string[] = [];
  let firstSaveStarted!: () => void;
  let releaseFirstSave!: () => void;
  const firstSaveStartedPromise = new Promise<void>((resolve) => {
    firstSaveStarted = resolve;
  });
  const releaseFirstSavePromise = new Promise<void>((resolve) => {
    releaseFirstSave = resolve;
  });

  const save = async (value: string) => {
    savedValues.push(value);
    if (savedValues.length === 1) {
      firstSaveStarted();
      await releaseFirstSavePromise;
    }
  };

  const first = queue.enqueue("9", save);
  await firstSaveStartedPromise;
  const second = queue.enqueue("10", save);
  const third = queue.enqueue("8", save);

  assert.deepEqual(savedValues, ["9"]);

  releaseFirstSave();
  await Promise.all([first, second, third]);

  assert.deepEqual(savedValues, ["9", "8"]);
});
