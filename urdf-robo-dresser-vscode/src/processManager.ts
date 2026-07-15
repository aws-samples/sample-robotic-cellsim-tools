/**
 * Injectable interfaces and data types for `RoboDresserProcessManager` — the
 * HTTP+process-spawn analogue of `StdioClient` (see `robo-fitting-room-vscode`'s
 * `stdioClient.ts`). Unlike `StdioClient`, the Robo Dresser server has no stdio
 * JSON-RPC transport: readiness and continued liveness are both determined by
 * polling HTTP endpoints (`/readyz`, `/healthz`) on a spawned `serve` process.
 *
 * This module defines the types/interfaces the design specifies for the
 * process manager, plus the `RoboDresserProcessManager` class itself
 * implementing the `ensureRunning`/`state`/`baseUrl`/`stop`/`openDocs` state
 * machine (the reachability monitor is implemented by task 6.1).
 */

import { isValidPort, firstInvalidPathSetting } from "./validators";

/** Req 3.1-3.6, 8.3: the process manager's state machine. Unlike
 * `RoboFitterServerManager`, `"failed"` is NOT terminal — the next
 * `ensureRunning` call after a failure always starts a fresh attempt. */
export type ServerState = "stopped" | "starting" | "ready" | "failed";

/** Req 5.1-5.4: options passed to `ensureRunning`, resolved from the
 * `roboDresser.*` configuration settings. */
export interface RoboDresserStartOptions {
  /** Resolved absolute path to the Catalogue directory. */
  catalogue: string;
  /** Validated 1-65535. */
  port: number;
  /** Non-empty overrides the bundled binary. */
  binaryPath?: string;
  /** Non-empty passed as `--web-root`. */
  webRoot?: string;
}

/** Injectable process spawner (child_process wrapper). */
export interface ProcessSpawner {
  spawn(binaryPath: string, args: readonly string[]): SpawnedProcess;
}

export interface SpawnedProcess {
  kill(): void;
  onExit(cb: (code: number | null) => void): void;
}

/** Injectable HTTP prober (fetch/http wrapper). */
export interface HttpProbe {
  /** GET the given path against the server's base URL; true iff it responds 2xx. */
  probe(baseUrl: string, path: "/readyz" | "/healthz"): Promise<boolean>;
}

export interface Clock {
  timeout(ms: number): Promise<void>;
  /** Recurring tick every `ms`; returns a handle to cancel it. */
  interval(ms: number, cb: () => void): { cancel(): void };
}

/** Distinct failure modes surfaced to the Reporter (Req 3.5, 5.8, 5.9, 8.3). */
export type StartFailureReason =
  | { kind: "invalid-port"; port: number }
  | { kind: "invalid-path"; setting: "catalogue" | "binaryPath" | "webRoot"; path: string }
  | { kind: "spawn-error"; message: string }
  | { kind: "timeout"; timeoutMs: number };

export interface Reporter {
  reportStartFailure(reason: StartFailureReason): void;
  reportBecameUnreachable(): void; // Req 8.4
  reportBecameReachableAgain(): void; // Req 8.5
  reportNoDocsUrl(): void;
  reportDocsUnreachable(): void;
}

/** Injectable browser-open function (`vscode.env.openExternal` wrapper), used by
 * {@link RoboDresserProcessManager.openDocs}. Kept synchronous-or-async so tests
 * can simulate success/failure either via a plain boolean return or a resolved/
 * rejected `Promise<boolean>` (Req 6.2-6.4). */
export type OpenExternal = (url: string) => boolean | Promise<boolean>;

export interface IRoboDresserProcessManager {
  /** Idempotent + retryable: `failed` is NOT terminal (unlike RoboFitterServerManager). */
  ensureRunning(options: RoboDresserStartOptions, timeoutMs: number): Promise<boolean>;
  /** Base URL of the running server, e.g. "http://127.0.0.1:3030". Undefined unless ready. */
  baseUrl(): string | undefined;
  /** Start continuous reachability polling (called once the viewer opens; Req 8.4, 8.5). */
  startReachabilityMonitor(intervalMs: number): void;
  stopReachabilityMonitor(): void;
  stop(): void;
  openDocs(): void;
  state(): ServerState;
}

/** Host/port/path configuration resolved before a start attempt (Req 5.1-5.4). */
export interface RoboDresserConfig {
  /** roboDresser.catalogue */
  catalogue: string;
  /** roboDresser.port, default 3030 (the server's own default, Req 5.2) */
  port: number;
  /** roboDresser.binaryPath, "" = use bundled binary */
  binaryPath: string;
  /** roboDresser.webRoot, "" = use embedded SPA */
  webRoot: string;
}

/** A single simulated/real poll outcome driving the reachability monitor. */
export type ReachabilityTick = "reachable" | "unreachable";

/**
 * `RoboDresserProcessManager` — implements the `stopped -> starting ->
 * {ready|failed}` state machine described in the design (Runtime flow:
 * launch / Key design decisions). Structurally mirrors
 * `RoboFitterServerManager`'s idempotent-`ensureRunning`-shares-in-flight-start
 * guarantee, with one deliberate divergence: `"failed"` is NOT terminal here
 * — the next `ensureRunning` call after a failed/timed-out attempt always
 * starts a brand-new attempt (Req 3.1).
 *
 * `ensureRunning`/`state`/`baseUrl`/`stop`/`openDocs` are implemented to spec
 * here; `startReachabilityMonitor` and `stopReachabilityMonitor` are left as
 * not-yet-implemented stubs — they are implemented properly by task 6.1.
 */

/** Injected dependencies for {@link RoboDresserProcessManager}. Kept fully
 * host-injectable (no direct `fs`/`child_process`/`fetch`/timer usage) so the
 * manager is unit-testable without real processes, real HTTP, or real time. */
export interface RoboDresserProcessManagerDeps {
  spawner: ProcessSpawner;
  probe: HttpProbe;
  clock: Clock;
  reporter: Reporter;
  /** Path-existence predicate used for Req 5.8 validation; keeps this module
   * free of a direct `fs` dependency. */
  exists: (path: string) => boolean;
  /** Absolute path to the binary bundled with the extension, used whenever
   * `RoboDresserStartOptions.binaryPath` is unset/empty. Resolving *which*
   * bundled binary to use (per-platform, `context.extensionPath`, etc.) is
   * host/runtime-specific and out of scope for this pure unit — callers
   * resolve it once and pass it in. */
  bundledBinaryPath: string;
  /** Opens a URL in the user's external browser (`vscode.env.openExternal`
   * wrapper), used by {@link RoboDresserProcessManager.openDocs} (Req 6.2-6.4). */
  openExternal: OpenExternal;
  /** Host the server binds to / is probed at. Defaults to `"127.0.0.1"`. */
  host?: string;
  /** Delay between `/readyz` poll attempts, in ms. Defaults to `250`. */
  readyPollIntervalMs?: number;
}

export class RoboDresserProcessManager implements IRoboDresserProcessManager {
  private readonly host: string;
  private readonly readyPollIntervalMs: number;

  private currentState: ServerState = "stopped";
  /** The single shared in-flight attempt while `currentState === "starting"`. */
  private inFlight: Promise<boolean> | undefined;
  private process: SpawnedProcess | undefined;
  /** Port of the currently-`ready` server; undefined unless `ready`. */
  private readyPort: number | undefined;
  /** Handle for the currently-running reachability interval, if any. */
  private reachabilityHandle: { cancel(): void } | undefined;
  /** Last-observed reachability tick; `undefined` means "unknown/not yet
   * observed" (monitor just started), so the very first `unreachable` outcome
   * always triggers a report even without a preceding `reachable` one. */
  private lastReachability: ReachabilityTick | undefined;

  constructor(private readonly deps: RoboDresserProcessManagerDeps) {
    this.host = deps.host ?? "127.0.0.1";
    this.readyPollIntervalMs = deps.readyPollIntervalMs ?? 250;
  }

  state(): ServerState {
    return this.currentState;
  }

  /**
   * Req 2.7, 3.1-3.6, 5.5, 5.8, 5.9, 8.3: drive the state machine.
   *
   * - `ready` -> resolves `true` immediately, no new spawn (Req 3.3/3.4).
   * - `starting` -> returns the SAME in-flight promise (Req 3.3): overlapping
   *   calls share one attempt, never spawning a second process.
   * - `stopped` or `failed` -> always starts a fresh attempt (Req 3.1): unlike
   *   `RoboFitterServerManager`, `failed` is NOT terminal.
   */
  ensureRunning(options: RoboDresserStartOptions, timeoutMs: number): Promise<boolean> {
    if (this.currentState === "ready") {
      return Promise.resolve(true);
    }
    if (this.currentState === "starting" && this.inFlight) {
      return this.inFlight;
    }

    const attempt = this.startAttempt(options, timeoutMs).finally(() => {
      // Only clear if this attempt is still the tracked one (it always is,
      // since a new attempt can't start until this one has settled out of
      // "starting").
      this.inFlight = undefined;
    });
    this.inFlight = attempt;
    return attempt;
  }

  /** Base URL of the running server. Undefined unless `ready` (per the
   * `IRoboDresserProcessManager.baseUrl` contract). */
  baseUrl(): string | undefined {
    if (this.currentState !== "ready" || this.readyPort === undefined) {
      return undefined;
    }
    return `http://${this.host}:${this.readyPort}`;
  }

  /** Kills the spawned process only if this manager started one; a no-op
   * otherwise (Req 3.8). Full nuance (e.g. reporting) belongs to task 5.4. */
  stop(): void {
    if (!this.process) {
      return;
    }
    this.process.kill();
    this.process = undefined;
    this.readyPort = undefined;
    this.currentState = "stopped";
  }

  /**
   * Req 6.2-6.4: while not `ready`, reports "no docs URL" and never invokes the
   * browser-open function. While `ready`, invokes the injected `openExternal`
   * with `<baseUrl>/docs` *regardless* of whether the server is actually
   * reachable (Req 6.2) — the docs URL is only what's opened, not verified. If
   * that call fails (throws synchronously, returns `false`, or returns a
   * promise that rejects or resolves `false`), reports "docs unreachable"
   * (Req 6.4).
   */
  openDocs(): void {
    if (this.currentState !== "ready") {
      this.deps.reporter.reportNoDocsUrl();
      return;
    }

    const docsUrl = `${this.baseUrl()}/docs`;
    let result: boolean | Promise<boolean>;
    try {
      result = this.deps.openExternal(docsUrl);
    } catch {
      this.deps.reporter.reportDocsUnreachable();
      return;
    }

    if (result instanceof Promise) {
      result
        .then((success) => {
          if (!success) {
            this.deps.reporter.reportDocsUnreachable();
          }
        })
        .catch(() => {
          this.deps.reporter.reportDocsUnreachable();
        });
      return;
    }

    if (!result) {
      this.deps.reporter.reportDocsUnreachable();
    }
  }

  /**
   * Req 8.4/8.5: start polling `/healthz` at intervals of at most
   * `intervalMs` via the injected `Clock.interval` (never a real timer).
   * Tracks the last-observed reachability tick, starting as "unknown" so the
   * very first `unreachable` outcome always reports (even without a
   * preceding `reachable` one). Reports exactly once per transition:
   * `reachable -> unreachable` (or unknown -> unreachable) calls
   * `reportBecameUnreachable()`; `unreachable -> reachable` calls
   * `reportBecameReachableAgain()`. Calling this while already running
   * cancels the previous interval first, so no interval is ever leaked.
   */
  startReachabilityMonitor(intervalMs: number): void {
    this.stopReachabilityMonitor();
    this.lastReachability = undefined;

    this.reachabilityHandle = this.deps.clock.interval(intervalMs, () => {
      void this.pollReachabilityOnce();
    });
  }

  stopReachabilityMonitor(): void {
    if (!this.reachabilityHandle) {
      return;
    }
    this.reachabilityHandle.cancel();
    this.reachabilityHandle = undefined;
  }

  /** One `/healthz` poll tick: skip gracefully if not `ready` (e.g. `stop()`
   * was called after the monitor started), otherwise probe and report on
   * transitions only. */
  private async pollReachabilityOnce(): Promise<void> {
    const baseUrl = this.baseUrl();
    if (baseUrl === undefined) {
      return;
    }

    const reachable = await this.deps.probe.probe(baseUrl, "/healthz");
    const tick: ReachabilityTick = reachable ? "reachable" : "unreachable";

    if (tick === "unreachable" && this.lastReachability !== "unreachable") {
      this.deps.reporter.reportBecameUnreachable();
      this.lastReachability = "unreachable";
    } else if (tick === "reachable" && this.lastReachability === "unreachable") {
      this.deps.reporter.reportBecameReachableAgain();
      this.lastReachability = "reachable";
    } else if (tick === "reachable") {
      this.lastReachability = "reachable";
    }
  }

  /**
   * One full start attempt: validate port, then paths (port first, per the
   * design's independent-of-path-priority note), spawn, then poll `/readyz`
   * until ready or `timeoutMs` elapses. On any failure, reports exactly one
   * `StartFailureReason` and settles into `failed`; leaves the viewer
   * unopened either way (opening the viewer is the caller's responsibility).
   */
  private async startAttempt(
    options: RoboDresserStartOptions,
    timeoutMs: number
  ): Promise<boolean> {
    this.currentState = "starting";

    if (!isValidPort(options.port)) {
      this.fail({ kind: "invalid-port", port: options.port });
      return false;
    }

    // Req 5.3/5.4: binaryPath/webRoot are only "set" when non-empty; an unset
    // binaryPath falls back to the bundled binary (a missing bundled binary
    // surfaces later as a spawn-error, not an invalid-path, per the design's
    // failure-reason table) and an unset webRoot is simply omitted from the
    // spawned args, so neither should be diagnosed as an invalid *setting*
    // when left empty.
    const resolvedBinaryPath =
      options.binaryPath && options.binaryPath.trim().length > 0
        ? options.binaryPath
        : this.deps.bundledBinaryPath;
    const resolvedWebRoot =
      options.webRoot && options.webRoot.trim().length > 0 ? options.webRoot : undefined;

    const existsOrUnset = (p: string): boolean => p === "" || this.deps.exists(p);
    const invalidSetting = firstInvalidPathSetting(
      {
        catalogue: options.catalogue,
        binaryPath: options.binaryPath ?? "",
        webRoot: options.webRoot ?? "",
      },
      existsOrUnset
    );
    if (invalidSetting) {
      const invalidPath =
        invalidSetting === "catalogue"
          ? options.catalogue
          : invalidSetting === "binaryPath"
            ? (options.binaryPath ?? "")
            : (options.webRoot ?? "");
      this.fail({ kind: "invalid-path", setting: invalidSetting, path: invalidPath });
      return false;
    }

    const args = ["serve", "--port", String(options.port), "--catalogue", options.catalogue];
    if (resolvedWebRoot) {
      args.push("--web-root", resolvedWebRoot);
    }

    let proc: SpawnedProcess;
    try {
      proc = this.deps.spawner.spawn(resolvedBinaryPath, args);
    } catch (e) {
      this.fail({ kind: "spawn-error", message: e instanceof Error ? e.message : String(e) });
      return false;
    }
    this.process = proc;

    const baseUrl = `http://${this.host}:${options.port}`;
    const ready = await this.pollUntilReady(baseUrl, timeoutMs);
    if (!ready) {
      this.fail({ kind: "timeout", timeoutMs });
      return false;
    }

    this.currentState = "ready";
    this.readyPort = options.port;
    return true;
  }

  /** Poll `/readyz` until it returns true or the `timeoutMs` budget (tracked
   * entirely via the injected `Clock`, never real timers) is exhausted. */
  private async pollUntilReady(baseUrl: string, timeoutMs: number): Promise<boolean> {
    let elapsedMs = 0;
    while (true) {
      const ready = await this.deps.probe.probe(baseUrl, "/readyz");
      if (ready) {
        return true;
      }
      if (elapsedMs >= timeoutMs) {
        return false;
      }
      const wait = Math.min(this.readyPollIntervalMs, timeoutMs - elapsedMs);
      await this.deps.clock.timeout(wait);
      elapsedMs += wait;
    }
  }

  /** Report exactly one failure reason and settle into `failed`. Reporter
   * exceptions are contained so they can never escape `ensureRunning`
   * (matching the containment guarantee task 5.5 tests explicitly). */
  private fail(reason: StartFailureReason): void {
    try {
      this.deps.reporter.reportStartFailure(reason);
    } catch {
      // Swallow: a throwing Reporter must not prevent reaching `failed`.
    } finally {
      this.currentState = "failed";
      this.process = undefined;
    }
  }
}
