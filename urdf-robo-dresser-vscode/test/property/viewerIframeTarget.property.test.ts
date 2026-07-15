/**
 * Property 14: Viewer iframe targets the configured server address.
 *
 * Targets `getRoboDresserViewerHtml(baseUrl)` (design.md "viewerHtml.ts" /
 * "Correctness Properties → Property 14"): for ANY valid host string and TCP
 * port in 1-65535, the generated HTML's iframe `src` attribute is exactly
 * `http://<host>:<port>/`, with no other panel content overriding or
 * duplicating that target — i.e. there is exactly one `<iframe>` tag in the
 * output and its `src` matches the expected URL exactly.
 *
 * Validates: Requirements 4.1
 */
import { describe, it, expect } from "vitest";
import fc from "fast-check";

import { getRoboDresserViewerHtml } from "../../src/viewerHtml";

/** Host generator: a small set of realistic hosts plus arbitrary hostname-like strings. */
const hostArb: fc.Arbitrary<string> = fc.oneof(
  fc.constantFrom("127.0.0.1", "localhost", "0.0.0.0", "192.168.1.10"),
  fc
    .array(fc.stringMatching(/^[a-z0-9]+$/), { minLength: 1, maxLength: 4 })
    .map((labels) => labels.join(".")),
);

const portArb: fc.Arbitrary<number> = fc.integer({ min: 1, max: 65535 });

/** Matches every `<iframe ...>` tag's `src="..."` attribute value. */
const IFRAME_SRC_RE = /<iframe\b[^>]*\bsrc="([^"]*)"[^>]*>/g;

/** Matches every `<iframe` opening tag (to count total iframe tags). */
const IFRAME_TAG_RE = /<iframe\b/g;

describe("Property 14: Viewer iframe targets the configured server address", () => {
  // Feature: robo-dresser-tool, Property 14: Viewer iframe targets the configured server address — for any valid host string and TCP port in 1-65535, the generated HTML's iframe src attribute is exactly http://<host>:<port>/, with no other panel content overriding or duplicating that target
  it("produces exactly one iframe whose src is exactly http://<host>:<port>/", () => {
    fc.assert(
      fc.property(hostArb, portArb, (host, port) => {
        const baseUrl = `http://${host}:${port}`;
        const expectedSrc = `${baseUrl}/`;

        const html = getRoboDresserViewerHtml(baseUrl);

        // Exactly one <iframe> tag in the whole document.
        const iframeTagMatches = html.match(IFRAME_TAG_RE) ?? [];
        expect(iframeTagMatches.length).toBe(1);

        // That single iframe's src is exactly the expected target — no other
        // panel content overrides or duplicates it.
        const srcMatches = [...html.matchAll(IFRAME_SRC_RE)];
        expect(srcMatches.length).toBe(1);
        expect(srcMatches[0][1]).toBe(expectedSrc);
      }),
      { numRuns: 100 },
    );
  });
});
