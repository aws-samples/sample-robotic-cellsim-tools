/**
 * Property-based test for `RoboDresserProcessManager.ensureRunning`/`state`.
 *
 * Feature: robo-dresser-tool, Property 13: Robo Dresser server start is idempotent and retryable — for any sequence of `ensureRunning` calls, including overlapping calls during an in-flight start and calls issued immediately after a prior failure or timeout: at most one process is spawned per attempt; the viewer-open callback fires at most once per attempt and only on a transition into `ready`; every failing/timed-out attempt produces exactly one start-failure report and never opens the viewer; and a `failed` state is never terminal, so the next call after a failure always starts a fresh attempt
 *
 * Validates: Requirements 2.7, 3.1, 3.2, 3.3, 3.5, 3.6, 8.3
 *
 * Adaptation note: this codebase's `ensureRunning` has no separate
 * "viewer-open callback" parameter — the caller opens the viewer itself,
 * only once `ensureRunning` resolves `true` (see `processManager.ts`'s
 * `startAttempt`, which only sets `currentState = "ready"` on the success
 * path, and `baseUrl()`, which is only defined while `ready`). This test
 * therefore exercises the equivalent *observable* contract directly: the
 * promise resolves `true` (and `baseUrl()` becomes defined) only on an
 * actual transition into `ready`, and resolves `false` (with `baseUrl()`
 * staying undefined) on every failure/timeout path — which is exactly what
 * would gate a caller's viewer-open callback in this design.
 *
 * Strategy: drive the manager through an arbitrary sequence of "rounds".
 * Each round picks an outcome (`success`, `timeout`, or `spawn-error`) and a
 * number of concurrent `ensureRunning` callers to fire before awaiting any
 * of them. A fake `Clock.timeout` resolves immediately (no real waiting) so
 * many iterations run fast and deterministically; a fake `HttpProbe.probe`
 * is driven purely by the current round's outcome. Concurrency is exercised
 * for `success`/`timeout` outcomes, since those are the only ones where the
 * attempt actually spans an `await` boundary (spawn succeeds synchronously,
 * then the attempt suspends at the first `probe`/`clock.timeout` await) —
 * i.e. the only outcomes where a burst of synchronous `ensureRunning` calls
 * can observe a genuinely in-flight `"starting"` attempt to share. A
 * `spawn-error` throws synchronously with no await point, so it settles to
 * `"failed"` within the same tick — there is no in-flight window to share,
 * so that outcome is exercised with a single caller per round (still fully
 * covering "exactly one spawn, exactly one failure report, resolves
 * false" for that failure mode). After every `success` round the fake
 * server is explicitly `stop()`-ed (reset to `"stopped"`) so the next round
 * starts a fresh, meaningfully-observable attempt; after every
 * failure/timeout round nothing is reset, which is precisely how the
 * "`failed` is never terminal" guarantee gets exercised.
 */
import { describe, it, expect } from "vitest";
import fc from "fast-check";

import {
  RoboDresserProcessManager,
  type RoboDresserProcessManagerDeps,
  type RoboDresserStartOptions,
  type SpawnedProcess,
  type StartFailureReason,
} from "../../src/processManager";

type RoundOutcome = "success" | "timeout" | "spawn-error";

interface Round {
  outcome: RoundOutcome;
  concurrentCalls: number;
}

const roundArbitrary: fc.Arbitrary<Round> = fc
  .record({
    outcome: fc.constantFrom<RoundOutcome>("success", "timeout", "spawn-error"),
    concurrentCalls: fc.integer({ min: 1, max: 5 }),
  })
  // spawn-error settles synchronously (no await point before failing), so a
  // burst of synchronous callers never observes a shared in-flight attempt
  // for it — see the file-level comment above.
  .map((r) => (r.outcome === "spawn-error" ? { ...r, concurrentCalls: 1 } : r));

const HOST = "127.0.0.1";
const PORT = 3030;
const TIMEOUT_MS = 500;

const START_OPTIONS: RoboDresserStartOptions = {
  catalogue: "/valid/catalogue",
  port: PORT,
};

/** Builds a fresh manager + its fakes/spies for a single property-test run. */
function buildManager(): {
  manager: RoboDresserProcessManager;
  setOutcome: (outcome: RoundOutcome) => void;
  spawnCallCount: () => number;
  reportCallCount: () => number;
  reportReasons: () => StartFailureReason[];
} {
  let currentOutcome: RoundOutcome = "success";
  let spawnCallCount = 0;
  let reportCallCount = 0;
  const reportReasons: StartFailureReason[] = [];

  const fakeProcess: SpawnedProcess = {
    kill: () => {},
    onExit: () => {},
  };

  const deps: RoboDresserProcessManagerDeps = {
    spawner: {
      spawn: () => {
        spawnCallCount++;
        if (currentOutcome === "spawn-error") {
          throw new Error("simulated spawn failure");
        }
        return fakeProcess;
      },
    },
    probe: {
      // "success": ready on the very first poll. "timeout": never ready, so
      // pollUntilReady exhausts its timeout budget and reports "timeout".
      probe: async () => currentOutcome === "success",
    },
    clock: {
      // No real timers: resolves immediately so the polling loop drives to
      // completion deterministically and fast regardless of numRuns.
      timeout: () => Promise.resolve(),
      interval: () => ({ cancel: () => {} }),
    },
    reporter: {
      reportStartFailure: (reason) => {
        reportCallCount++;
        reportReasons.push(reason);
      },
      reportBecameUnreachable: () => {},
      reportBecameReachableAgain: () => {},
      reportNoDocsUrl: () => {},
      reportDocsUnreachable: () => {},
    },
    exists: () => true, // path validity is Property 15's concern, not this one's
    bundledBinaryPath: "/bin/urdf-robo-dresser",
    host: HOST,
  };

  const manager = new RoboDresserProcessManager(deps);

  return {
    manager,
    setOutcome: (outcome) => {
      currentOutcome = outcome;
    },
    spawnCallCount: () => spawnCallCount,
    reportCallCount: () => reportCallCount,
    reportReasons: () => reportReasons,
  };
}

describe("RoboDresserProcessManager — Property 13: server start is idempotent and retryable", () => {
  it(
    "spawns at most once per attempt, reports exactly one failure per failing/timed-out " +
      "attempt, resolves true only on a real transition into ready, and always starts a " +
      "fresh attempt after a failure since failed is never terminal",
    async () => {
      return fc.assert(
        fc.asyncProperty(
          fc.array(roundArbitrary, { minLength: 1, maxLength: 8 }),
          async (rounds) => {
            const { manager, setOutcome, spawnCallCount, reportCallCount } = buildManager();

            for (const round of rounds) {
              setOutcome(round.outcome);

              const spawnCountBefore = spawnCallCount();
              const reportCountBefore = reportCallCount();

              // Fire all concurrent callers synchronously, before awaiting
              // any of them, so overlapping calls genuinely race against the
              // same in-flight attempt rather than being serialized by the
              // test.
              const promises: Promise<boolean>[] = [];
              for (let i = 0; i < round.concurrentCalls; i++) {
                promises.push(manager.ensureRunning(START_OPTIONS, TIMEOUT_MS));
              }

              const resolved = await Promise.all(promises);

              const spawnDelta = spawnCallCount() - spawnCountBefore;
              const reportDelta = reportCallCount() - reportCountBefore;

              // At most (exactly) one process spawned per attempt, no matter
              // how many overlapping callers asked for it.
              expect(spawnDelta).toBe(1);

              if (round.outcome === "success") {
                // Every caller observes the same successful transition into
                // ready — the observable analogue of "the viewer-open
                // callback fires at most once per attempt and only on a
                // transition into ready".
                expect(resolved.every((r) => r === true)).toBe(true);
                expect(manager.state()).toBe("ready");
                expect(manager.baseUrl()).toBe(`http://${HOST}:${PORT}`);
                expect(reportDelta).toBe(0);

                // Reset so the next round starts a fresh, observable attempt.
                manager.stop();
                expect(manager.state()).toBe("stopped");
              } else {
                // Every failing/timed-out attempt: every caller resolves
                // false, never opens the viewer (baseUrl stays undefined),
                // and produces exactly one start-failure report.
                expect(resolved.every((r) => r === false)).toBe(true);
                expect(manager.state()).toBe("failed");
                expect(manager.baseUrl()).toBeUndefined();
                expect(reportDelta).toBe(1);

                // Deliberately do NOT reset: the next round's ensureRunning
                // call must start a fresh attempt on its own, proving
                // "failed" is not terminal.
              }
            }
          }
        ),
        { numRuns: 100 }
      );
    }
  );
});
