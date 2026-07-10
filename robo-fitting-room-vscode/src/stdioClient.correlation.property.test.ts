// Property-based test for StdioClient's id-correlation logic.
//
// Property 4: Id correlation is a bijection over the request's lifetime.
// For an arbitrary interleaving of N concurrent requests and their
// responses (arriving in arbitrary order, including out-of-order), each
// request's promise resolves with EXACTLY its own corresponding response
// and no other — a bijection between request ids and settled responses.
// Unknown/duplicate response ids never settle anything erroneously.
//
// **Validates: Requirements 6.1, 6.2, 6.4, 6.6**
//
// StdioClient normally spawns a real child process, which is impractical to
// property-test. We instead inject a fake ChildProcess-like object (an
// EventEmitter with a capturing `stdin.write` and an `exitCode` of `null`)
// into the client's `proc` field, then drive the *real* correlation code:
// `request()` (which assigns ids, writes NDJSON lines, and registers pending
// entries) and `handleStdoutChunk()` (which buffers/splits stdout and settles
// pending entries by id). No production method is stubbed.

import { EventEmitter } from "events";
import { describe, it, expect, vi } from "vitest";
import * as fc from "fast-check";

// StdioClient's constructor calls into the `vscode` API (output channel and
// status bar item). `vscode` is an ambient module only present inside the
// extension host, so we mock the surface the constructor touches.
vi.mock("vscode", () => {
  const outputChannel = {
    appendLine: vi.fn(),
    append: vi.fn(),
    show: vi.fn(),
    dispose: vi.fn(),
  };
  const statusBarItem = {
    text: "",
    tooltip: "",
    command: "",
    show: vi.fn(),
    dispose: vi.fn(),
  };
  return {
    window: {
      createOutputChannel: vi.fn(() => outputChannel),
      createStatusBarItem: vi.fn(() => statusBarItem),
      showErrorMessage: vi.fn(),
      showInformationMessage: vi.fn(),
      withProgress: vi.fn(
        async (_opts: unknown, task: () => Promise<unknown>) => task()
      ),
      showOpenDialog: vi.fn(),
      ProgressLocation: { Notification: 15 },
    },
    StatusBarAlignment: { Left: 1, Right: 2 },
    workspace: {
      getConfiguration: vi.fn(() => ({
        get: (_key: string, dflt: unknown) => dflt,
        update: vi.fn(),
      })),
      workspaceFolders: undefined,
    },
    ConfigurationTarget: { Workspace: 2 },
    Uri: { file: (p: string) => ({ fsPath: p }) },
  };
});

import { StdioClient } from "./stdioClient";

/** A minimal stdin stand-in that records every written frame. */
class FakeStdin extends EventEmitter {
  public readonly writes: string[] = [];
  write(chunk: string): boolean {
    this.writes.push(chunk);
    return true; // never signals backpressure in this test
  }
}

/** A minimal ChildProcess stand-in sufficient for request()/handleStdoutChunk. */
class FakeChildProcess extends EventEmitter {
  public readonly stdout = new EventEmitter();
  public readonly stderr = new EventEmitter();
  public readonly stdin = new FakeStdin();
  public exitCode: number | null = null;
  kill(): boolean {
    this.exitCode = 0;
    return true;
  }
}

/**
 * Build a client with an injected running fake process, and wire the fake's
 * stdout to the real `handleStdoutChunk` so responses flow through the exact
 * production path a live child would use.
 */
function makeClient(): { client: StdioClient; proc: FakeChildProcess } {
  const client = new StdioClient();
  const proc = new FakeChildProcess();
  // Reach past `private` (compile-time only) to inject the fake process.
  const internal = client as unknown as {
    proc: FakeChildProcess;
    handleStdoutChunk: (buf: Buffer) => void;
  };
  internal.proc = proc;
  proc.stdout.on("data", (d: Buffer) => internal.handleStdoutChunk(d));
  return { client, proc };
}

/** Feed one NDJSON response frame through the fake child's stdout. */
function emitResponse(proc: FakeChildProcess, obj: unknown): void {
  proc.stdout.emit("data", Buffer.from(JSON.stringify(obj) + "\n"));
}

/** Derive an arbitrary permutation of [0..n) from a list of sort keys. */
function permutation(sortKeys: number[], n: number): number[] {
  return Array.from({ length: n }, (_, i) => i).sort(
    (a, b) => sortKeys[a] - sortKeys[b] || a - b
  );
}

describe("Property 4: id correlation is a bijection over the request's lifetime", () => {
  it("every request resolves with exactly its own response, regardless of arrival order, ignoring unknown/duplicate ids", async () => {
    await fc.assert(
      fc.asyncProperty(
        // Arbitrary method names for N concurrent requests (1..12).
        fc.array(fc.string(), { minLength: 1, maxLength: 12 }),
        // Sort keys used to derive an arbitrary response arrival order.
        fc.array(fc.integer(), { minLength: 12, maxLength: 12 }),
        // Whether to also feed a response for an id nobody is awaiting.
        fc.boolean(),
        // Whether to also feed a duplicate of an already-settled response.
        fc.boolean(),
        async (methods, sortKeys, includeUnknown, includeDuplicate) => {
          const { client, proc } = makeClient();
          const n = methods.length;

          // Issue N concurrent requests. Each write is captured synchronously
          // and in order, so writes[i] correlates to request i.
          const promises = methods.map((m, i) =>
            (client as unknown as {
              request: (method: string, params: unknown) => Promise<unknown>;
            }).request(m, { index: i })
          );

          const sent = proc.stdin.writes.map((w) => JSON.parse(w) as {
            id: number;
            method: string;
            params: { index: number };
          });

          // Sanity: one frame written per request, ids unique.
          expect(sent).toHaveLength(n);
          const ids = sent.map((s) => s.id);
          expect(new Set(ids).size).toBe(n);

          // Each request gets a uniquely identifiable payload.
          const responses = sent.map((s, i) => ({
            id: s.id,
            result: { echo: i, method: s.method },
          }));

          const order = permutation(sortKeys, n);

          // A response for an id nobody is tracking must be silently ignored.
          if (includeUnknown) {
            const unknownId = Math.max(...ids) + 1000;
            expect(() =>
              emitResponse(proc, { id: unknownId, result: { bogus: true } })
            ).not.toThrow();
          }

          // Deliver responses in the arbitrary permuted order.
          for (const idx of order) {
            emitResponse(proc, responses[idx]);
          }

          // A duplicate of an already-settled response must be ignored, never
          // double-settling a promise or throwing.
          if (includeDuplicate) {
            expect(() =>
              emitResponse(proc, responses[order[0]])
            ).not.toThrow();
          }

          // Bijection: every promise settles, each with its OWN payload.
          const results = await Promise.all(promises);
          for (let i = 0; i < n; i++) {
            expect(results[i]).toEqual({ echo: i, method: sent[i].method });
          }

          client.dispose();
        }
      ),
      { numRuns: 200 }
    );
  });
});
