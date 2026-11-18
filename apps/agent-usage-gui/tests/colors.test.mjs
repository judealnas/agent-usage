import assert from "node:assert/strict";
import test from "node:test";
import { usageHue } from "../src/colors.ts";

test("gradient reaches yellow halfway to the configured red threshold", () => {
  for (const threshold of [1, 50, 90, 100]) {
    assert.equal(usageHue(0, threshold), 120);
    assert.equal(usageHue(threshold / 2, threshold), 60);
    assert.equal(usageHue(threshold, threshold), 0);
    assert.equal(usageHue(120, threshold), 0);
  }
});

test("color varies continuously and clamps values outside the range", () => {
  assert.ok(Math.abs(usageHue(45.01, 90) - usageHue(45, 90)) < 0.02);
  assert.equal(usageHue(-10, 90), 120);
  assert.equal(usageHue(NaN, 90), 120);
  assert.equal(usageHue(45, 0), 60);
});
