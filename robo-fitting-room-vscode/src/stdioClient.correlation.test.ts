// Unit tests for StdioClient's response/id correlation codec.
//
// These exercise handleStdoutChunk/handleLine + request() without spawning a
// real child process: a fake child is injected into the private `proc`, its
// stdin captures written request lines, and its stdout is an EventEmitter we
// drive to deliver response frames. This covers (Requirements 6.3, 6.4):
//   - out-of-order arrivals each settle their own promise,
//   - a response for an unknown id is silently ignored,
//   - a duplicate response for an already-settled id is silently ignored,
//   - and (bonus) a frame split across a chunk boundary is reassembled.
//
// `stdioClient.ts` imports `vscode`, which only exists inside the VS Code
// extension host, so the whole module is mocked with just the two window
// factories the constructor touches.

import { EventEmitter } from "events";
import { describe, it, expect, beforeEach, vi } from "vitest";

vi.mock("vscode", () => {
  const makeDisposable = () => ({
    text: "",
    tooltip: "",
    command: "",
    show: vi.fn(),
    hide: vi.fn(),
    dispose: vi.fn(),
    appendLine: vi.fn(),
    append: vi.fn(),
  });
  return {
    window: {
      createOutputChannel: vi.fn(() => makeDisposable()),
      createStatusBarItem: vi.fn(() => makeDisposable()),
    },
    StatusBarAlignment: { Left: 1, Right: 2 },
    ProgressLocation: { Notification: 15 },
    ConfigurationTarget: { Workspace: 2 },
  };
});

import { StdioClient, StdioRequestError } from "./stdioClient";

/** A fake ChildProcess exposing just what StdioClient touches. */
interface FakeProc {
  stdout: EventEmitter;
  stderr: EventEmitter;
  stdin: { write: (line: string) => boolean; on: () => void; removeAllListeners: () => void };
  written: string[];
  exitCode: number | null;
  on: () => void;
  removeAllListeners: () => void;
  kill: () => void;
}

function makeFakeProc(): FakeProc {
  const stdout = new EventEmitter();
  const stderr = new EventEmitter();
  const written: string[] = [];
  return {
    stdout,
    stderr,
    stdin: {
      write: (line: string) => {
        written.push(line);
        return true;
      },
      on: () => {},
      removeAllListeners: () => {},
    },
    written,
    exitCode: null,
    on: () => {},
    removeAllListeners: () => {},
    kill: () => {},
  };
}

/**
 * Build a StdioClient with a fake child injected as if `start()` had spawned
 * it: `proc` is set and the fake stdout is wired to the private stdout-chunk
 * handler so `stdout.emit("data", ...)` drives the codec.
 */
function makeClient(): { client: StdioClient; proc: FakeProc } {
  const client = new StdioClient();
  const proc = makeFakeProc();
  // Cast to any to set the private `proc` field and reach private handlers.
  (client as any).proc = proc;
  proc.stdout.on("data", (d: Buffer) => (client as any).handleStdoutChunk(d));
  return { client, proc };
}

/** Emit one complete NDJSON response frame on the fake child's stdout. */
function emitFrame(proc: FakeProc, obj: unknown): void {
  proc.stdout.emit("data", Buffer.from(JSON.stringify(obj) + "\n"));
}

/** The `id` assigned to the request whose line was captured at `index`. */
function writtenId(proc: FakeProc, index: number): number {
  return JSON.parse(proc.written[index]).id as number;
}

/** Track promise settlement without awaiting it inline. */
function track<T>(p: Promise<T>): { settled: boolean; value?: T; error?: unknown } {
  const state: { settled: boolean; value?: T; error?: unknown } = { settled: false };
  p.then(
    (v) => {
      state.settled = true;
      state.value = v;
    },
    (e) => {
      state.settled = true;
      state.error = e;
    }
  );
  return state;
}

/** Let queued microtasks/timers flush so tracked promises settle. */
function flush(): Promise<void> {
  return new Promise((r) => setTimeout(r, 0));
}

describe("StdioClient response correlation (Requirements 6.3, 6.4)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("out-of-order responses each settle their own originating promise", async () => {
    const { client, proc } = makeClient();

    const p1 = client.request<unknown, string>("m1", { n: 1 });
    const p2 = client.request<unknown, string>("m2", { n: 2 });
    const p3 = client.request<unknown, string>("m3", { n: 3 });

    // Three distinct ids were assigned and written, one per request.
    expect(proc.written).toHaveLength(3);
    const id1 = writtenId(proc, 0);
    const id2 = writtenId(proc, 1);
    const id3 = writtenId(proc, 2);
    expect(new Set([id1, id2, id3]).size).toBe(3);

    // Deliver responses in a scrambled order: 3, then 1, then 2.
    emitFrame(proc, { id: id3, result: "r3" });
    emitFrame(proc, { id: id1, result: "r1" });
    emitFrame(proc, { id: id2, result: "r2" });

    // Each promise resolves with its OWN matching result regardless of order.
    await expect(p1).resolves.toBe("r1");
    await expect(p2).resolves.toBe("r2");
    await expect(p3).resolves.toBe("r3");

    // Everything settled; nothing left pending.
    expect((client as any).pending.size).toBe(0);
  });

  it("a response for an unknown id is ignored and leaves pending requests untouched", async () => {
    const { client, proc } = makeClient();

    const p1 = client.request<unknown, string>("m1", {});
    const known = writtenId(proc, 0);
    const s1 = track(p1);

    // A response for an id that was never requested must not throw and must
    // not settle the concurrently-pending request.
    expect(() => emitFrame(proc, { id: known + 999, result: "ghost" })).not.toThrow();
    await flush();

    expect(s1.settled).toBe(false);
    expect((client as any).pending.size).toBe(1);

    // The genuine response still settles its own promise correctly.
    emitFrame(proc, { id: known, result: "real" });
    await expect(p1).resolves.toBe("real");
    expect((client as any).pending.size).toBe(0);
  });

  it("an unknown-id response does not disturb an already-settled request", async () => {
    const { client, proc } = makeClient();

    const p1 = client.request<unknown, string>("m1", {});
    const id1 = writtenId(proc, 0);

    emitFrame(proc, { id: id1, result: "done" });
    await expect(p1).resolves.toBe("done");

    // A later response for an id nobody tracks is silently dropped.
    expect(() => emitFrame(proc, { id: 4242, result: "late-ghost" })).not.toThrow();
    await flush();
    expect((client as any).pending.size).toBe(0);
  });

  it("a duplicate response for an already-settled id is silently ignored", async () => {
    const { client, proc } = makeClient();

    const p1 = client.request<unknown, string>("m1", {});
    const id1 = writtenId(proc, 0);

    // First response settles the promise.
    emitFrame(proc, { id: id1, result: "first" });
    await expect(p1).resolves.toBe("first");
    expect((client as any).pending.size).toBe(0);

    // A duplicate response for the same, now-settled id must be ignored: no
    // throw, and the promise keeps its first resolved value (a promise can
    // only settle once anyway, but the codec must not error either).
    expect(() => emitFrame(proc, { id: id1, result: "second" })).not.toThrow();
    await flush();
    await expect(p1).resolves.toBe("first");
    expect((client as any).pending.size).toBe(0);
  });

  it("a duplicate error response after a resolved id is also ignored", async () => {
    const { client, proc } = makeClient();

    const p1 = client.request<unknown, string>("m1", {});
    const id1 = writtenId(proc, 0);

    emitFrame(proc, { id: id1, result: "ok" });
    await expect(p1).resolves.toBe("ok");

    // Even a differently-shaped (error) duplicate must not re-settle or throw.
    expect(() =>
      emitFrame(proc, { id: id1, error: { code: 500, message: "boom" } })
    ).not.toThrow();
    await flush();
    await expect(p1).resolves.toBe("ok");
  });

  it("an error response rejects with StdioRequestError carrying the rpc error", async () => {
    const { client, proc } = makeClient();

    const p1 = client.request<unknown, string>("m1", {});
    const id1 = writtenId(proc, 0);

    emitFrame(proc, {
      id: id1,
      error: { code: 404, message: "not found", data: { code: "missing" } },
    });

    await expect(p1).rejects.toBeInstanceOf(StdioRequestError);
    await p1.catch((e: StdioRequestError) => {
      expect(e.rpcError.code).toBe(404);
      expect(e.rpcError.data?.code).toBe("missing");
    });
  });

  it("reassembles a response frame split across chunk boundaries", async () => {
    const { client, proc } = makeClient();

    const p1 = client.request<unknown, string>("m1", {});
    const id1 = writtenId(proc, 0);

    const frame = JSON.stringify({ id: id1, result: "reassembled" }) + "\n";
    const cut = Math.floor(frame.length / 2);

    // Deliver the frame in two arbitrary pieces; the second piece carries the
    // trailing newline that completes the line.
    proc.stdout.emit("data", Buffer.from(frame.slice(0, cut)));
    await flush();
    // Still pending: no newline seen yet.
    expect((client as any).pending.size).toBe(1);

    proc.stdout.emit("data", Buffer.from(frame.slice(cut)));
    await expect(p1).resolves.toBe("reassembled");
    expect((client as any).pending.size).toBe(0);
  });
});
