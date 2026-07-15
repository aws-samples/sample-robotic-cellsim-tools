/**
 * Property-based test for the pure `firstInvalidPathSetting` validator.
 *
 * Feature: robo-dresser-tool, Property 15: Invalid-path reporting follows a fixed priority order — for any combination of validity assigned independently to `catalogue`, `binaryPath`, `webRoot`, the setting reported as invalid is exactly the first of those three, in that order, whose assigned validity is "does not exist"; when all three are valid, no invalid-path failure is reported
 *
 * Validates: Requirements 5.8
 *
 * Strategy: generate three independent booleans (catalogueExists,
 * binaryPathExists, webRootExists) covering all eight combinations of
 * validity across the three settings. Build an `exists` predicate mock that
 * returns exactly those booleans for the three arbitrary (but distinct)
 * path strings assigned to the settings — the specific path values don't
 * matter, only which setting's path the predicate reports as missing. The
 * oracle independently computes the expected first-invalid setting in fixed
 * priority order (catalogue, then binaryPath, then webRoot), or `undefined`
 * when all three are valid, and compares it against
 * `firstInvalidPathSetting`'s actual return value.
 */
import { describe, it, expect } from "vitest";
import fc from "fast-check";

import { firstInvalidPathSetting } from "../../src/validators";

describe("firstInvalidPathSetting — Property 15: invalid-path reporting follows a fixed priority order", () => {
  it("reports exactly the first of catalogue/binaryPath/webRoot (in that order) whose validity is false, or undefined when all are valid", () => {
    fc.assert(
      fc.property(
        fc.boolean(),
        fc.boolean(),
        fc.boolean(),
        (catalogueExists, binaryPathExists, webRootExists) => {
          const settings = {
            catalogue: "/paths/catalogue",
            binaryPath: "/paths/binaryPath",
            webRoot: "/paths/webRoot",
          };

          const existsByPath: Record<string, boolean> = {
            [settings.catalogue]: catalogueExists,
            [settings.binaryPath]: binaryPathExists,
            [settings.webRoot]: webRootExists,
          };

          const exists = (p: string): boolean => existsByPath[p];

          const expected = !catalogueExists
            ? "catalogue"
            : !binaryPathExists
              ? "binaryPath"
              : !webRootExists
                ? "webRoot"
                : undefined;

          expect(firstInvalidPathSetting(settings, exists)).toBe(expected);
        }
      ),
      { numRuns: 100 }
    );
  });
});
