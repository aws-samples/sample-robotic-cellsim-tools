/**
 * Property-based test for the pure `isValidPort` validator.
 *
 * Feature: robo-dresser-tool, Property 16: Port validity gates server start — for any integer assigned to `roboDresser.port`, a start attempt proceeds to spawn a process if and only if the value is within 1-65535 inclusive
 *
 * Validates: Requirements 5.9
 *
 * Strategy: generate arbitrary integers spanning well below 1 and well above
 * 65535 (in addition to the valid range) so both boundaries and the
 * out-of-range regions on either side are exercised. The oracle is the
 * mathematical definition of port validity (`port >= 1 && port <= 65535`),
 * checked independently of the implementation, and compared against
 * `isValidPort`'s actual return value.
 */
import { describe, it, expect } from "vitest";
import fc from "fast-check";

import { isValidPort } from "../../src/validators";

describe("isValidPort — Property 16: port validity gates server start", () => {
  it("returns true iff the integer is within 1-65535 inclusive", () => {
    fc.assert(
      fc.property(fc.integer({ min: -100_000, max: 200_000 }), (port) => {
        const expected = port >= 1 && port <= 65535;
        expect(isValidPort(port)).toBe(expected);
      }),
      { numRuns: 200 }
    );
  });
});
