import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import { afterAll, beforeAll, describe, it, expect, vi } from "vitest";

/**
 * Cross-transport integration test: spawns the *actual* built
 * `robo-fitting-room` binary in `stdio-serve` mode and drives the real
 * {@link StdioClient} over its stdin/stdout pipes — no fakes, no mocked child
 * process. It exercises three things end-to-end:
 *
 *   1. the `health.check` handshake performed by `start()`/`ensureRunning()`
 *      (Req 5.2 — readiness within a bounded timeout),
 *   2. a normal request (`assets.listRobots`) round-tripping through the NDJSON
 *      codec to a sensible result (Req 6.2 — id-correlated resolution),
 *   3. a deliberate crash: the child is SIGKILL'd while a request is in flight,
 *      and the still-pending request must reject (Req 5.3, 6.2 — every in-flight
 *      request rejected with a descriptive error when the process dies).
 *
 * `StdioClient` does `import * as vscode from "vscode"`, which has no runtime
 * module outside the extension host, and its constructor / `start()` reach into
 * `vscode.window` + `vscode.workspace`. We mock only that surface so the client
 * runs in a plain Node/vitest process:
 *   - `getConfiguration().get` returns the real built-binary path as
 *     `binaryPath` and a real fixture dir as `assetRoot`, so no picker dialog is
 *     needed,
 *   - `withProgress` simply runs its task callback (so the handshake actually
 *     executes),
 *   - `showErrorMessage` / `showInformationMessage` are no-ops.
 */

// Absolute path to the debug binary built with the `stdio` feature
// (`cargo build --features stdio`), resolved relative to this test file so it
// is independent of the working directory vitest is launched from.
const BINARY_PATH = path.resolve(
  __dirname,
  "../../robo-fitting-room/target/debug/robo-fitting-room"
);
const BINARY_EXISTS = fs.existsSync(BINARY_PATH);

// Mutable holder read by the mocked `vscode` configuration. Hoisted so the
// (hoisted) `vi.mock` factory below can close over it; populated in beforeAll
// once the fixture dir exists.
const mockCfg = vi.hoisted(() => ({
  binaryPath: "",
  assetRoot: "",
  webRoot: "",
}));

vi.mock("vscode", () => {
  const makeOutputChannel = () => ({
    appendLine: vi.fn(),
    append: vi.fn(),
    show: vi.fn(),
    dispose: vi.fn(),
  });
  const makeStatusBarItem = () => ({
    text: "",
    tooltip: "",
    command: "",
    show: vi.fn(),
    dispose: vi.fn(),
  });
  return {
    StatusBarAlignment: { Left: 1, Right: 2 },
    ProgressLocation: { Notification: 15 },
    ConfigurationTarget: { Global: 1, Workspace: 2, WorkspaceFolder: 3 },
    window: {
      createOutputChannel: vi.fn(makeOutputChannel),
      createStatusBarItem: vi.fn(makeStatusBarItem),
      showErrorMessage: vi.fn(),
      showInformationMessage: vi.fn(),
      // Run the task so the real handshake request executes and its readiness
      // result is returned verbatim.
      withProgress: vi.fn(
        async (_opts: unknown, task: () => Promise<unknown>) => task()
      ),
    },
    workspace: {
      getConfiguration: vi.fn(() => ({
        get: (key: string, dflt: unknown) =>
          (mockCfg as Record<string, unknown>)[key] ?? dflt,
        update: vi.fn(),
      })),
      workspaceFolders: undefined,
    },
  };
});

import { StdioClient, StdioProcessError } from "./stdioClient";

/**
 * Lay down the minimal asset tree `assets.listRobots` needs: a dressed-robot
 * dir with a matching `<id>.urdf` (the lister only checks the file exists — it
 * does not parse it), so the normal request returns a non-empty result.
 */
function populateAssets(root: string): void {
  const robotDir = path.join(root, "urdf-dressed-robots", "robot1");
  fs.mkdirSync(robotDir, { recursive: true });
  fs.writeFileSync(path.join(robotDir, "robot1.urdf"), '<?xml version="1.0"?>\n<robot name="robot1"/>\n');
}

let fixtureDir: string;

describe.skipIf(!BINARY_EXISTS)(
  "StdioClient ⇄ real stdio-serve binary (integration)",
  () => {
    beforeAll(() => {
      fixtureDir = fs.mkdtempSync(path.join(os.tmpdir(), "rfr-stdio-it-"));
      populateAssets(fixtureDir);
      mockCfg.binaryPath = BINARY_PATH;
      mockCfg.assetRoot = fixtureDir;
      mockCfg.webRoot = "";
    });

    afterAll(() => {
      if (fixtureDir) {
        fs.rmSync(fixtureDir, { recursive: true, force: true });
      }
    });

    it(
      "handshakes on start, serves a normal request, and rejects in-flight requests when the child crashes",
      async () => {
        const client = new StdioClient();
        try {
          // (1) Handshake — start() spawns the binary and awaits health.check.
          const ready = await client.ensureRunning();
          expect(ready).toBe(true);
          expect(client.isRunning).toBe(true);

          // (2) Normal request — assets.listRobots round-trips to a sensible
          // result (the one seeded robot), proving id-correlation works over
          // the real pipes.
          const robots = await client.request<
            Record<string, never>,
            Array<{ id: string; urdfRel: string }>
          >("assets.listRobots", {});
          expect(Array.isArray(robots)).toBe(true);
          expect(robots.map((r) => r.id)).toContain("robot1");

          // (3) Deliberate crash mid-request. Issue a request so a pending
          // entry is registered, then drive the client's own termination path
          // deterministically instead of racing a real (fast) RPC against an
          // out-of-band SIGKILL.
          //
          // `client.request(...)` registers its pending entry synchronously (the
          // Promise executor runs before the call returns). Synchronously
          // emitting the child's `'exit'` event runs the same `handleTermination`
          // path an unexpected crash takes — rejecting every in-flight request —
          // in this same turn of the event loop, *before* Node can deliver any
          // stdout response (child stdout is delivered on a future tick). That
          // removes the timing race while still validating the real crash
          // behavior: in-flight requests reject with a descriptive
          // StdioProcessError and no pending entries are orphaned (Req 5.3, 6.2).
          //
          // We then actually SIGKILL the real child so the deterministic
          // synthetic exit doesn't leave an orphaned process behind.
          const proc = (
            client as unknown as {
              proc: {
                emit(event: string, ...args: unknown[]): boolean;
                kill(sig: string): void;
              };
            }
          ).proc;
          const inFlight = client.request("assets.listRobots", {});
          // Synchronously trigger the unexpected-exit path (code=null,
          // signal=SIGKILL) before any stdout response microtask can resolve it.
          proc.emit("exit", null, "SIGKILL");
          // Reap the real child so it doesn't outlive the test.
          proc.kill("SIGKILL");

          await expect(inFlight).rejects.toBeInstanceOf(StdioProcessError);
          await expect(inFlight).rejects.toThrow(
            /exit|error|before responding|disposed/i
          );

          // The crash marked the client stopped and left no orphaned pending
          // entries.
          expect(client.isRunning).toBe(false);
          expect(
            (client as unknown as { pending: Map<number, unknown> }).pending.size
          ).toBe(0);

          // A subsequent request on the dead client is refused up front.
          await expect(client.request("assets.listRobots", {})).rejects.toThrow(
            /no running server/i
          );
        } finally {
          client.dispose();
        }
      },
      30000
    );
  }
);
