/**
 * Property-based test for `RoboDresserProcessManager.startReachabilityMonitor`/
 * `stopReachabilityMonitor`.
 *
 * Feature: robo-dresser-tool, Property 17: Reachability-monitoring liveness (both directions) —
 * for any sequence of reachability poll outcomes (`reachable` / `unreachable`) observed while the
 * viewer is open, with polls occurring at intervals of at most 10 seconds: an unreachable
 * indication is shown within one poll interval of the first `unreachable` outcome following a
 * `reachable` one, and, symmetrically, that indication is updated or removed within one poll
 * interval of the first `reachable` outcome following an `unreachable` one.
 *
 * Validates: Requirements 8.4, 8.5
 *
 * Strategy: drive the manager to `"ready"` via a successful `ensureRunning` (fake spawner/probe/
 * clock, mirroring `startIdempotency.property.test.ts`), then call `startReachabilityMonitor`
 * with a fake `Clock.interval` that captures its tick callback instead of using a real timer.
 * For an arbitrary sequence of `"reachable" | "unreachable"` outcomes, each element is fed to the
 * fake `HttpProbe.probe` and the captured tick callback is invoked once per element (simulating
 * one poll interval elapsing). After each tick, `reportBecameUnreachable`/
 * `reportBecameReachableAgain` call counts are asserted to change by exactly 1 iff this tick is
 * the first `unreachable`/`reachable` outcome since the last opposite-direction outcome (or since
 * monitor start), and by 0 otherwise — this is the "within one poll interval" liveness bound
 * expressed as an exact per-tick transition report, since each tick call *is* one poll interval
 * elapsing in this synchronous test harness.
 *
 * `stopReachabilityMonitor()` is exercised at the end of every run by asserting it invokes
 * `cancel()` on the interval handle exactly once. Per the real (non-test) `Clock.interval`
 * contract, `cancel()` is what stops future ticks from ever firing — `pollReachabilityOnce`
 * itself has no separate "stopped" guard beyond `baseUrl()` being defined, so a manually
 * re-invoked stale tick callback would not be a faithful simulation of "stopped"; verifying
 * `cancel()` fired is the correct proxy for "no further reports occur once stopped".
 */
import { describe, it, expect } from "vitest";
import fc from "fast-check";

import {
  RoboDresserProcessManager,
  type RoboDresserProcessManagerDeps,
  type RoboDresserStartOptions,
  type SpawnedProcess,
} from "../../src/processManager";

type ReachabilityTick = "reachable" | "unreachable";

const HOST = "127.0.0.1";
const PORT = 3030;
const TIMEOUT_MS = 500;
const INTERVAL_MS = 10_000; // "at most 10 seconds" per the property's own bound

const START_OPTIONS: RoboDresserStartOptions = {
  catalogue: "/valid/catalogue",
  port: PORT,
};

/** Builds a fresh manager wired to `"ready"`-capable fakes, plus a capturable fake
 * `Clock.interval` and report-call spies, for a single property-test run. */
function buildManager(): {
  manager: RoboDresserProcessManager;
  setProbeOutcome: (reachable: boolean) => void;
  getCapturedTick: () => (() => void) | undefined;
  cancelCallCount: () => number;
  unreachableCount: () => number;
  reachableAgainCount: () => number;
} {
  let probeReachable = true;
  let capturedTick: (() => void) | undefined;
  let cancelCallCount = 0;
  let unreachableCount = 0;
  let reachableAgainCount = 0;

  const fakeProcess: SpawnedProcess = {
    kill: () => {},
    onExit: () => {},
  };

  const deps: RoboDresserProcessManagerDeps = {
    spawner: {
      spawn: () => fakeProcess,
    },
    probe: {
      // `/readyz` (used during ensureRunning) always succeeds immediately;
      // `/healthz` (used by the reachability monitor) is driven by
      // `probeReachable`, set per-tick by the test.
      probe: async (_baseUrl, path) => (path === "/readyz" ? true : probeReachable),
    },
    clock: {
      timeout: () => Promise.resolve(),
      interval: (_ms, cb) => {
        capturedTick = cb;
        return {
          cancel: () => {
            cancelCallCount++;
            capturedTick = undefined;
          },
        };
      },
    },
    reporter: {
      reportStartFailure: () => {},
      reportBecameUnreachable: () => {
        unreachableCount++;
      },
      reportBecameReachableAgain: () => {
        reachableAgainCount++;
      },
      reportNoDocsUrl: () => {},
      reportDocsUnreachable: () => {},
    },
    exists: () => true,
    bundledBinaryPath: "/bin/urdf-robo-dresser",
    host: HOST,
  };

  const manager = new RoboDresserProcessManager(deps);

  return {
    manager,
    setProbeOutcome: (reachable) => {
      probeReachable = reachable;
    },
    getCapturedTick: () => capturedTick,
    cancelCallCount: () => cancelCallCount,
    unreachableCount: () => unreachableCount,
    reachableAgainCount: () => reachableAgainCount,
  };
}

const tickArbitrary = fc.constantFrom<ReachabilityTick>("reachable", "unreachable");

describe("RoboDresserProcessManager — Property 17: reachability-monitoring liveness (both directions)", () => {
  it(
    "reports becameUnreachable/becameReachableAgain exactly on the first tick of each " +
      "direction-transition, and never reports after stopReachabilityMonitor",
    async () => {
      await fc.assert(
        fc.asyncProperty(
          fc.array(tickArbitrary, { minLength: 1, maxLength: 20 }),
          async (ticks) => {
            const {
              manager,
              setProbeOutcome,
              getCapturedTick,
              cancelCallCount,
              unreachableCount,
              reachableAgainCount,
            } = buildManager();

            const ready = await manager.ensureRunning(START_OPTIONS, TIMEOUT_MS);
            expect(ready).toBe(true);
            expect(manager.state()).toBe("ready");

            manager.startReachabilityMonitor(INTERVAL_MS);
            const tick = getCapturedTick();
            expect(tick).toBeDefined();

            // "unknown" until the first observed outcome, per the manager's
            // own documented semantics: the very first `unreachable` tick
            // always reports even with no preceding `reachable` one.
            let lastDirection: ReachabilityTick | undefined = undefined;

            for (const outcome of ticks) {
              setProbeOutcome(outcome === "reachable");

              const unreachableBefore = unreachableCount();
              const reachableAgainBefore = reachableAgainCount();

              tick!();
              // Let the async pollReachabilityOnce (probe await) settle.
              await Promise.resolve();
              await Promise.resolve();

              const unreachableDelta = unreachableCount() - unreachableBefore;
              const reachableAgainDelta = reachableAgainCount() - reachableAgainBefore;

              const isFirstUnreachableSinceReachable =
                outcome === "unreachable" && lastDirection !== "unreachable";
              const isFirstReachableSinceUnreachable =
                outcome === "reachable" && lastDirection === "unreachable";

              expect(unreachableDelta).toBe(isFirstUnreachableSinceReachable ? 1 : 0);
              expect(reachableAgainDelta).toBe(isFirstReachableSinceUnreachable ? 1 : 0);

              lastDirection = outcome;
            }

            // Stopping the monitor must cancel the interval handle exactly
            // once — the real mechanism by which the injected `Clock`
            // guarantees no further ticks (and therefore no further
            // reports) ever fire again.
            const cancelCountBefore = cancelCallCount();
            manager.stopReachabilityMonitor();
            expect(cancelCallCount()).toBe(cancelCountBefore + 1);
          }
        ),
        { numRuns: 100 }
      );
    }
  );
});
