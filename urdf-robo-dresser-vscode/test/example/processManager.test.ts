/**
 * Example-based unit tests for `RoboDresserProcessManager.stop()`,
 * `.openDocs()`, and reporter-exception containment inside `ensureRunning`
 * (task 5.5).
 *
 * Covers:
 *   - `stop()` is a no-op when no process was ever spawned (Req 3.8)
 *   - `openDocs()` while `ready` always calls `openExternal` with
 *     `<baseUrl>/docs`, regardless of a simulated reachability flag,
 *     since there is no separate reachability check before calling it
 *     (Req 6.2)
 *   - `openDocs()` while not `ready` reports "no docs URL" and never calls
 *     `openExternal` (Req 6.3)
 *   - a failing `openExternal` (thrown, resolved `false`, or rejected)
 *     while `ready` reports "docs unreachable" — a distinct reporter method
 *     from "no docs URL" (Req 6.4)
 *   - a `Reporter.reportStartFailure` that throws does not escape
 *     `ensureRunning`; the manager still settles into `failed` (Req 3.9)
 *
 * _Requirements: 3.8, 3.9, 6.2, 6.3, 6.4_
 */

import { describe, it, expect, vi } from "vitest";

import {
  RoboDresserProcessManager,
  type RoboDresserProcessManagerDeps,
  type RoboDresserStartOptions,
  type SpawnedProcess,
} from "../../src/processManager";

const HOST = "127.0.0.1";
const PORT = 3030;
const TIMEOUT_MS = 500;

const START_OPTIONS: RoboDresserStartOptions = {
  catalogue: "/valid/catalogue",
  port: PORT,
};

/** Builds a fresh manager + its fakes/spies for a single test. `openExternal`
 * defaults to a successful no-op spy; individual tests override it via
 * `deps.openExternal` reassignment isn't possible (readonly-ish usage), so
 * callers instead pass an `openExternal` override through `buildManager`. */
function buildManager(overrides?: {
  openExternal?: RoboDresserProcessManagerDeps["openExternal"];
  reportStartFailure?: RoboDresserProcessManagerDeps["reporter"]["reportStartFailure"];
  probeReady?: boolean;
}): {
  manager: RoboDresserProcessManager;
  reportNoDocsUrl: ReturnType<typeof vi.fn>;
  reportDocsUnreachable: ReturnType<typeof vi.fn>;
  openExternal: ReturnType<typeof vi.fn>;
  spawnCallCount: () => number;
} {
  let spawnCallCount = 0;

  const fakeProcess: SpawnedProcess = {
    kill: vi.fn(),
    onExit: () => {},
  };

  const reportNoDocsUrl = vi.fn();
  const reportDocsUnreachable = vi.fn();
  const openExternal =
    overrides?.openExternal !== undefined
      ? (vi.fn(overrides.openExternal) as ReturnType<typeof vi.fn>)
      : (vi.fn(() => true) as ReturnType<typeof vi.fn>);

  const deps: RoboDresserProcessManagerDeps = {
    spawner: {
      spawn: () => {
        spawnCallCount++;
        return fakeProcess;
      },
    },
    probe: {
      probe: async () => overrides?.probeReady ?? true,
    },
    clock: {
      timeout: () => Promise.resolve(),
      interval: () => ({ cancel: () => {} }),
    },
    reporter: {
      reportStartFailure: overrides?.reportStartFailure ?? (() => {}),
      reportBecameUnreachable: () => {},
      reportBecameReachableAgain: () => {},
      reportNoDocsUrl,
      reportDocsUnreachable,
    },
    exists: () => true,
    bundledBinaryPath: "/bin/urdf-robo-dresser",
    openExternal: openExternal as unknown as RoboDresserProcessManagerDeps["openExternal"],
    host: HOST,
  };

  const manager = new RoboDresserProcessManager(deps);

  return {
    manager,
    reportNoDocsUrl,
    reportDocsUnreachable,
    openExternal,
    spawnCallCount: () => spawnCallCount,
  };
}

describe("RoboDresserProcessManager.stop() (example)", () => {
  it("is a no-op when ensureRunning never succeeded: no kill(), no throw, stays stopped (Req 3.8)", () => {
    const { manager } = buildManager();

    expect(manager.state()).toBe("stopped");
    expect(() => manager.stop()).not.toThrow();
    expect(manager.state()).toBe("stopped");
  });
});

describe("RoboDresserProcessManager.openDocs() (example)", () => {
  it(
    "while ready, calls openExternal with `<baseUrl>/docs` regardless of a simulated " +
      "reachability flag (Req 6.2)",
    async () => {
      const { manager, openExternal } = buildManager();

      const started = await manager.ensureRunning(START_OPTIONS, TIMEOUT_MS);
      expect(started).toBe(true);
      expect(manager.state()).toBe("ready");

      manager.openDocs();

      expect(openExternal).toHaveBeenCalledTimes(1);
      expect(openExternal).toHaveBeenCalledWith(`${manager.baseUrl()}/docs`);
    }
  );

  it("while stopped (never started), reports no-docs-url and never calls openExternal (Req 6.3)", () => {
    const { manager, reportNoDocsUrl, openExternal } = buildManager();

    expect(manager.state()).toBe("stopped");
    manager.openDocs();

    expect(reportNoDocsUrl).toHaveBeenCalledTimes(1);
    expect(openExternal).not.toHaveBeenCalled();
  });

  it("after a failure, reports no-docs-url and never calls openExternal (Req 6.3)", async () => {
    const { manager, reportNoDocsUrl, openExternal } = buildManager({
      probeReady: false, // never ready -> pollUntilReady times out -> failed
    });

    const started = await manager.ensureRunning(START_OPTIONS, 0);
    expect(started).toBe(false);
    expect(manager.state()).toBe("failed");

    manager.openDocs();

    expect(reportNoDocsUrl).toHaveBeenCalledTimes(1);
    expect(openExternal).not.toHaveBeenCalled();
  });

  it("after stop(), reports no-docs-url and never calls openExternal (Req 6.3)", async () => {
    const { manager, reportNoDocsUrl, openExternal } = buildManager();

    await manager.ensureRunning(START_OPTIONS, TIMEOUT_MS);
    manager.stop();
    expect(manager.state()).toBe("stopped");

    manager.openDocs();

    expect(reportNoDocsUrl).toHaveBeenCalledTimes(1);
    expect(openExternal).not.toHaveBeenCalled();
  });

  it(
    "a synchronously-throwing openExternal while ready reports docs-unreachable, distinct " +
      "from no-docs-url (Req 6.4)",
    async () => {
      const { manager, reportNoDocsUrl, reportDocsUnreachable } = buildManager({
        openExternal: () => {
          throw new Error("simulated openExternal failure");
        },
      });

      await manager.ensureRunning(START_OPTIONS, TIMEOUT_MS);
      expect(manager.state()).toBe("ready");

      expect(() => manager.openDocs()).not.toThrow();

      expect(reportDocsUnreachable).toHaveBeenCalledTimes(1);
      expect(reportNoDocsUrl).not.toHaveBeenCalled();
    }
  );

  it(
    "an openExternal returning false while ready reports docs-unreachable, distinct from " +
      "no-docs-url (Req 6.4)",
    async () => {
      const { manager, reportNoDocsUrl, reportDocsUnreachable } = buildManager({
        openExternal: () => false,
      });

      await manager.ensureRunning(START_OPTIONS, TIMEOUT_MS);
      expect(manager.state()).toBe("ready");

      manager.openDocs();

      expect(reportDocsUnreachable).toHaveBeenCalledTimes(1);
      expect(reportNoDocsUrl).not.toHaveBeenCalled();
    }
  );

  it(
    "a rejecting-promise openExternal while ready reports docs-unreachable, distinct from " +
      "no-docs-url (Req 6.4)",
    async () => {
      const { manager, reportNoDocsUrl, reportDocsUnreachable } = buildManager({
        openExternal: () => Promise.reject(new Error("simulated async failure")),
      });

      await manager.ensureRunning(START_OPTIONS, TIMEOUT_MS);
      expect(manager.state()).toBe("ready");

      manager.openDocs();
      // The rejection is handled asynchronously inside openDocs; flush microtasks.
      await Promise.resolve();
      await Promise.resolve();

      expect(reportDocsUnreachable).toHaveBeenCalledTimes(1);
      expect(reportNoDocsUrl).not.toHaveBeenCalled();
    }
  );

  it(
    "a promise-resolving-false openExternal while ready reports docs-unreachable, distinct " +
      "from no-docs-url (Req 6.4)",
    async () => {
      const { manager, reportNoDocsUrl, reportDocsUnreachable } = buildManager({
        openExternal: () => Promise.resolve(false),
      });

      await manager.ensureRunning(START_OPTIONS, TIMEOUT_MS);
      expect(manager.state()).toBe("ready");

      manager.openDocs();
      await Promise.resolve();
      await Promise.resolve();

      expect(reportDocsUnreachable).toHaveBeenCalledTimes(1);
      expect(reportNoDocsUrl).not.toHaveBeenCalled();
    }
  );
});

describe("RoboDresserProcessManager reporter-exception containment (example)", () => {
  it(
    "a Reporter.reportStartFailure that throws does not escape ensureRunning; the manager " +
      "still settles into failed (Req 3.9)",
    async () => {
      const { manager } = buildManager({
        reportStartFailure: () => {
          throw new Error("simulated reporter failure");
        },
        // Invalid port -> fails validation before any spawn/poll, exercising
        // the reporter-throwing path deterministically and synchronously.
      });

      const invalidPortOptions: RoboDresserStartOptions = {
        catalogue: "/valid/catalogue",
        port: 0, // invalid: outside 1-65535
      };

      // If the reporter's exception escaped ensureRunning, this await would
      // throw and fail the test.
      const resolved = await manager.ensureRunning(invalidPortOptions, TIMEOUT_MS);

      expect(resolved).toBe(false);
      expect(manager.state()).toBe("failed");
    }
  );
});
