/**
 * Integration test for `serve` reaching `/readyz` within the 30s timeout
 * budget on a valid catalogue, and the viewer panel's iframe target being
 * reachable (task 16.1).
 *
 * Exercises the REAL `urdf-robo-dresser` Rust binary (built via
 * `cargo build` in the sibling `urdf-robo-dresser` crate) through the REAL
 * `RoboDresserProcessManager.ensureRunning`, wired to real
 * `child_process.spawn`-backed `ProcessSpawner` and real `http.get`-backed
 * `HttpProbe` implementations (mirroring `RealProcessSpawner`/
 * `RealHttpProbe` in `src/extension.ts`) — the actual HTTP roundtrip against
 * the actually-spawned process is what determines readiness, nothing about
 * `/readyz` itself is faked. Only the `Clock`'s poll *delay* uses a real
 * (but short, 75ms) interval so the test polls aggressively instead of
 * waiting out anywhere near the real 30s budget; the 30s value itself is
 * still passed through to `ensureRunning` unmodified, so this test confirms
 * that value is correctly threaded through rather than actually waiting for
 * it.
 *
 * Also renders `getRoboDresserViewerHtml(baseUrl)` and issues a real HTTP
 * GET against the iframe's `src` (`<baseUrl>/`) as a pragmatic proxy for
 * "the viewer panel's iframe successfully loads the SPA root" — a full
 * browser-based iframe-load assertion isn't feasible in a Node test
 * environment.
 *
 * _Requirements: 3.2, 4.1_
 */

import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import * as http from "http";
import { spawn, type ChildProcess } from "child_process";

import { describe, it, expect, afterEach } from "vitest";

import {
  RoboDresserProcessManager,
  type ProcessSpawner,
  type SpawnedProcess,
  type HttpProbe,
  type Clock,
  type Reporter,
  type RoboDresserStartOptions,
} from "../../src/processManager";
import { getRoboDresserViewerHtml } from "../../src/viewerHtml";

/** Req 3.2/8.3: the production start-timeout value. Passed through
 * unmodified to `ensureRunning` so the test confirms it is correctly wired,
 * without actually waiting anywhere near 30s in practice (aggressive 75ms
 * polling below means real readiness is observed in well under a second). */
const START_TIMEOUT_MS = 30_000;

const CRATE_ROOT = path.resolve(__dirname, "..", "..", "..", "urdf-robo-dresser");
const BINARY_NAME = process.platform === "win32" ? "urdf-robo-dresser.exe" : "urdf-robo-dresser";
const BINARY_PATH = path.join(CRATE_ROOT, "target", "debug", BINARY_NAME);

const NOOP_REPORTER: Reporter = {
  reportStartFailure: () => {},
  reportBecameUnreachable: () => {},
  reportBecameReachableAgain: () => {},
  reportNoDocsUrl: () => {},
  reportDocsUnreachable: () => {},
};

/** Real `child_process.spawn`-backed {@link ProcessSpawner}, mirroring
 * `RealProcessSpawner` in `src/extension.ts`. Every spawned child is also
 * tracked in `spawnedProcesses` so `afterEach` can force-kill it even if the
 * manager's own bookkeeping ever loses track of the handle. */
const spawnedProcesses: ChildProcess[] = [];

class RealProcessSpawner implements ProcessSpawner {
  spawn(binaryPath: string, args: readonly string[]): SpawnedProcess {
    const proc = spawn(binaryPath, args as string[]);
    spawnedProcesses.push(proc);
    return {
      kill: () => proc.kill(),
      onExit: (cb) => proc.on("exit", (code) => cb(code)),
    };
  }
}

/** Real HTTP-backed {@link HttpProbe}, mirroring `RealHttpProbe` in
 * `src/extension.ts`: the actual roundtrip to the spawned server is what
 * determines the probe result. */
class RealHttpProbe implements HttpProbe {
  probe(baseUrl: string, probePath: "/readyz" | "/healthz"): Promise<boolean> {
    return new Promise<boolean>((resolve) => {
      const req = http.get(`${baseUrl}${probePath}`, (res) => {
        const status = res.statusCode ?? 0;
        res.resume();
        resolve(status >= 200 && status < 300);
      });
      req.on("error", () => resolve(false));
      req.setTimeout(2000, () => {
        req.destroy();
        resolve(false);
      });
    });
  }
}

/** Real `setTimeout`/`setInterval`-backed {@link Clock}. Combined with a
 * short `readyPollIntervalMs` (see manager construction below), this keeps
 * the poll *delay* real but short, instead of mocking away the passage of
 * time entirely. */
class RealClock implements Clock {
  timeout(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms));
  }
  interval(ms: number, cb: () => void): { cancel(): void } {
    const handle = setInterval(cb, ms);
    return { cancel: () => clearInterval(handle) };
  }
}

/** Minimal single-link URDFs — enough for `catalogue::scan` to count as a
 * robot/tool (making `/readyz` report ready) without needing mesh files,
 * matching the fixture shape used by the crate's own `tests/server.rs`. */
function seedCatalogue(root: string): void {
  const robotDir = path.join(root, "urdf-robot-collection", "r1");
  fs.mkdirSync(robotDir, { recursive: true });
  fs.writeFileSync(
    path.join(robotDir, "r1.urdf"),
    '<?xml version="1.0"?>\n<robot name="r1">\n  <link name="base"/>\n</robot>\n'
  );

  const toolDir = path.join(root, "urdf-tool-collection", "t1");
  fs.mkdirSync(toolDir, { recursive: true });
  fs.writeFileSync(
    path.join(toolDir, "t1.urdf"),
    '<?xml version="1.0"?>\n<robot name="t1">\n  <link name="tcp"/>\n</robot>\n'
  );
}

/** Binds an ephemeral port on 127.0.0.1 momentarily to get a free port, then
 * releases it immediately for the real binary to bind. Avoids collisions
 * with the default 3030 (or other concurrently-running instances). */
async function findFreePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = http.createServer();
    server.on("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      if (address && typeof address === "object") {
        const { port } = address;
        server.close(() => resolve(port));
      } else {
        server.close();
        reject(new Error("failed to allocate a free port"));
      }
    });
  });
}

function httpGetStatus(url: string): Promise<number> {
  return new Promise((resolve, reject) => {
    const req = http.get(url, (res) => {
      res.resume();
      resolve(res.statusCode ?? 0);
    });
    req.on("error", reject);
    req.setTimeout(2000, () => {
      req.destroy();
      reject(new Error(`timed out GETting ${url}`));
    });
  });
}

describe("serve reaching /readyz + viewer iframe target reachability (integration)", () => {
  let tmpCatalogueDir: string | undefined;
  let manager: RoboDresserProcessManager | undefined;

  afterEach(() => {
    manager?.stop();
    manager = undefined;

    for (const proc of spawnedProcesses) {
      if (!proc.killed) {
        proc.kill();
      }
    }
    spawnedProcesses.length = 0;

    if (tmpCatalogueDir) {
      fs.rmSync(tmpCatalogueDir, { recursive: true, force: true });
      tmpCatalogueDir = undefined;
    }
  });

  it(
    "real urdf-robo-dresser serve becomes ready within the 30s budget on a valid " +
      "catalogue, and the viewer's iframe target responds successfully (Req 3.2, 4.1)",
    async () => {
      if (!fs.existsSync(BINARY_PATH)) {
        throw new Error(
          `expected a built urdf-robo-dresser debug binary at ${BINARY_PATH}. ` +
            `Run \`cargo build\` in ${CRATE_ROOT} first.`
        );
      }

      tmpCatalogueDir = fs.mkdtempSync(path.join(os.tmpdir(), "robo-dresser-catalogue-"));
      seedCatalogue(tmpCatalogueDir);

      const port = await findFreePort();

      manager = new RoboDresserProcessManager({
        spawner: new RealProcessSpawner(),
        probe: new RealHttpProbe(),
        clock: new RealClock(),
        reporter: NOOP_REPORTER,
        exists: fs.existsSync,
        bundledBinaryPath: BINARY_PATH,
        openExternal: () => true,
        host: "127.0.0.1",
        // Poll aggressively (every 75ms) so real readiness is observed
        // quickly rather than waiting anywhere close to the 30s budget.
        readyPollIntervalMs: 75,
      });

      const options: RoboDresserStartOptions = { catalogue: tmpCatalogueDir, port };

      const started = await manager.ensureRunning(options, START_TIMEOUT_MS);

      expect(started).toBe(true);
      expect(manager.state()).toBe("ready");

      const baseUrl = manager.baseUrl();
      expect(baseUrl).toBe(`http://127.0.0.1:${port}`);

      // Req 4.1: the viewer's generated iframe src must target the SPA root.
      const html = getRoboDresserViewerHtml(baseUrl!);
      const iframeMatch = html.match(/<iframe[^>]*\ssrc="([^"]+)"/);
      expect(iframeMatch).not.toBeNull();
      const iframeSrc = iframeMatch![1];
      expect(iframeSrc).toBe(`${baseUrl}/`);

      // Pragmatic proxy for "the viewer panel's iframe successfully loads
      // the SPA root": a real HTTP GET against the iframe's own target URL
      // succeeds.
      const status = await httpGetStatus(iframeSrc);
      expect(status).toBeGreaterThanOrEqual(200);
      expect(status).toBeLessThan(300);
    },
    10_000
  );
});
