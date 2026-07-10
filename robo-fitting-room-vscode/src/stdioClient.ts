import * as vscode from "vscode";
import { ChildProcess, spawn } from "child_process";
import * as path from "path";
import * as fs from "fs";

/** Bounded timeout for the initial `health.check` handshake on spawn. */
const HANDSHAKE_TIMEOUT_MS = 5000;
/** Default per-request timeout used by {@link StdioClient.request}. */
const DEFAULT_REQUEST_TIMEOUT_MS = 30000;

/** Error object shape carried in an RPC `error` response. */
export interface RpcError {
  code: number;
  message: string;
  data?: { code: string; hint?: string };
}

/** Thrown when a request resolves to an RPC `error` response. */
export class StdioRequestError extends Error {
  constructor(public readonly rpcError: RpcError) {
    super(rpcError.message);
    this.name = "StdioRequestError";
  }
}

/**
 * Thrown into a still-pending request when the managed child process
 * terminates (crash, unexpected exit, spawn error) or the client is disposed
 * before that request received a response.
 */
export class StdioProcessError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "StdioProcessError";
  }
}

/** A request awaiting its correlated response. */
interface PendingEntry {
  resolve: (value: unknown) => void;
  reject: (err: Error) => void;
  method: string;
}

/**
 * Owns the lifecycle of a `robo-fitting-room stdio-serve` child process and
 * communicates with it over newline-delimited JSON on stdin/stdout. Replaces
 * `ServerManager`'s HTTP-based lifecycle: no TCP port is opened, and readiness
 * is confirmed with a single `health.check` handshake instead of polling
 * `/healthz`.
 *
 * This module implements the process spawn, handshake, lifecycle
 * (`ensureRunning`/`start`/`stop`/`dispose`), status-bar behavior, and the full
 * request/id-correlation codec: stdout chunk buffering with `\n`
 * line-splitting, a monotonically increasing `nextId`, a `pending` map keyed by
 * id, and per-request timeout rejection.
 *
 * Writes to the child's stdin are backpressure-aware: {@link enqueueWrite}
 * tracks `stdin.write()`'s boolean return and, once the stream reports it is
 * full (or a queue has already formed), buffers subsequent lines in
 * {@link writeQueue}, flushing them on the stream's `'drain'` event. On
 * `'exit'`/`'error'` every in-flight request in {@link pending} is rejected,
 * `proc` and the write queue are cleared, and the status bar returns to
 * `stopped`; {@link dispose} does the same after killing the child.
 */
export class StdioClient implements vscode.Disposable {
  private proc: ChildProcess | undefined;
  private readonly pending = new Map<number, PendingEntry>();
  private nextId = 1;
  private lineBuffer = "";
  /** Lines buffered while stdin reports backpressure; flushed on `'drain'`. */
  private writeQueue: string[] = [];
  /** True while a `stdin.write()` has returned `false` and we await `'drain'`. */
  private draining = false;
  /** True once {@link dispose} has run; guards post-dispose status updates. */
  private disposed = false;
  private readonly output: vscode.OutputChannel;
  private readonly status: vscode.StatusBarItem;
  /**
   * Absolute path to the extension's install directory (from
   * `context.extensionPath`), used to locate the binary bundled under
   * `<extensionPath>/bin/`. Optional so the client can still be constructed
   * in unit tests without a full extension context.
   */
  private readonly extensionPath: string | undefined;

  constructor(extensionPath?: string) {
    this.extensionPath = extensionPath;
    this.output = vscode.window.createOutputChannel("robo-fitting-room");
    this.status = vscode.window.createStatusBarItem(
      vscode.StatusBarAlignment.Left,
      100
    );
    this.status.command = "roboFittingRoom.openEditor";
    this.updateStatus("stopped");
  }

  /**
   * Resolve the `robo-fitting-room` binary to spawn, in priority order:
   *   1. an explicit `roboFittingRoom.binaryPath` setting (absolute path or a
   *      name resolved against `PATH`) — lets users point at their own build;
   *   2. the binary bundled inside this (architecture-specific) `.vsix` at
   *      `<extensionPath>/bin/robo-fitting-room[.exe]` — the default for a
   *      packaged install, made executable on first use;
   *   3. a bare `"robo-fitting-room"` looked up on `PATH` — a graceful
   *      fallback when neither of the above is available (e.g. running from
   *      source without a bundled binary).
   */
  private resolveBinary(): string {
    const configured = cfg().get<string>("binaryPath", "").trim();

    // Only treat the setting as a deliberate override when it names a concrete
    // location (absolute, or a relative path with a separator). A bare name
    // like the legacy default "robo-fitting-room" must NOT shadow the bundled
    // binary — otherwise a leftover setting sends us to a PATH lookup that
    // usually isn't there, and the process never starts.
    if (
      configured &&
      (path.isAbsolute(configured) || configured.includes(path.sep))
    ) {
      return configured;
    }

    // Prefer the binary bundled inside this architecture-specific .vsix.
    const bundled = this.bundledBinaryPath();
    if (bundled) {
      return bundled;
    }

    // Last resort: a name on PATH (the configured bare name, or the default).
    return configured || "robo-fitting-room";
  }

  /**
   * Absolute path to the bundled `bin/robo-fitting-room[.exe]` if it exists,
   * made executable first (VS Code does not always preserve the +x bit when it
   * unpacks a `.vsix`, so re-assert it). Returns `undefined` when running from
   * source with no bundled binary.
   */
  private bundledBinaryPath(): string | undefined {
    if (!this.extensionPath) {
      return undefined;
    }
    const exe =
      process.platform === "win32"
        ? "robo-fitting-room.exe"
        : "robo-fitting-room";
    const p = path.join(this.extensionPath, "bin", exe);
    if (!fs.existsSync(p)) {
      return undefined;
    }
    if (process.platform !== "win32") {
      try {
        fs.chmodSync(p, 0o755);
      } catch {
        /* best-effort; spawn will surface a real permission error */
      }
    }
    return p;
  }

  /** True when a managed child process is spawned and has not exited. */
  get isRunning(): boolean {
    return this.proc !== undefined && this.proc.exitCode === null;
  }

  private updateStatus(state: "running" | "stopped"): void {
    if (this.disposed) {
      return;
    }
    if (state === "running") {
      this.status.text = "$(vm-active) robo-fitting-room";
      this.status.tooltip = "robo-fitting-room process running";
    } else {
      this.status.text = "$(vm-outline) robo-fitting-room";
      this.status.tooltip = "Process not running — click to open the editor";
    }
    this.status.show();
  }

  /**
   * Ensure a managed process is running and ready. If one is already running,
   * resolves `true` immediately; otherwise spawns and handshakes.
   */
  async ensureRunning(): Promise<boolean> {
    if (this.isRunning) {
      return true;
    }
    return this.start();
  }

  /**
   * Spawn `<binaryPath> stdio-serve --asset-root <root> [--web-root <root>]`
   * (no host/port) and perform the `health.check` handshake before declaring
   * the process ready.
   */
  async start(): Promise<boolean> {
    if (this.isRunning) {
      this.output.appendLine("Process already managed by this window.");
      return true;
    }

    const assetRoot = await this.resolveAssetRoot();
    if (!assetRoot) {
      return false;
    }

    const c = cfg();
    const bin = this.resolveBinary();
    const webRoot = c.get<string>("webRoot", "").trim();

    const args = ["stdio-serve", "--asset-root", assetRoot];
    if (webRoot) {
      args.push("--web-root", webRoot);
    }

    this.output.show(true);
    this.output.appendLine(`$ ${bin} ${args.join(" ")}`);

    // Fresh process ⇒ fresh write-queue state; nothing carries over from a
    // previous (crashed/stopped) child.
    this.writeQueue = [];
    this.draining = false;

    try {
      this.proc = spawn(bin, args, { cwd: assetRoot });
    } catch (e) {
      vscode.window.showErrorMessage(`Failed to spawn process: ${errMsg(e)}`);
      return false;
    }

    this.proc.stdout?.on("data", (d: Buffer) => this.handleStdoutChunk(d));
    this.proc.stderr?.on("data", (d: Buffer) => this.output.append(d.toString()));
    // Stdin drained: resume flushing any lines queued during backpressure.
    this.proc.stdin?.on("drain", () => this.flushWriteQueue());
    this.proc.on("error", (e) => {
      this.output.appendLine(`\n[error] ${e.message}`);
      vscode.window.showErrorMessage(
        `robo-fitting-room process error: ${e.message}. Check "roboFittingRoom.binaryPath".`
      );
      this.handleTermination(
        new StdioProcessError(`robo-fitting-room process error: ${e.message}`)
      );
    });
    this.proc.on("exit", (code, signal) => {
      this.output.appendLine(`\n[process exited] code=${code} signal=${signal}`);
      this.handleTermination(
        new StdioProcessError(
          `robo-fitting-room process exited (code=${code ?? "null"}, signal=${signal ?? "null"}) before responding`
        )
      );
    });

    // Readiness is a single handshake request, not a poll loop: the child
    // either answers on its first line or it's broken.
    const ready = await vscode.window.withProgress(
      {
        location: vscode.ProgressLocation.Notification,
        title: "Starting robo-fitting-room process…",
      },
      async () => {
        try {
          await this.request("health.check", {}, HANDSHAKE_TIMEOUT_MS);
          return true;
        } catch (e) {
          this.output.appendLine(`[handshake failed] ${errMsg(e)}`);
          return false;
        }
      }
    );

    if (ready) {
      this.updateStatus("running");
      this.output.appendLine("Process ready (health.check ok).");
    } else {
      vscode.window.showErrorMessage(
        "Process did not become ready in time. See the robo-fitting-room output channel."
      );
    }
    return ready;
  }

  /** Terminate the managed process (no-op if none is running). */
  stop(): void {
    if (!this.isRunning) {
      vscode.window.showInformationMessage("No managed process to stop.");
      return;
    }
    this.output.appendLine("Stopping managed process…");
    this.proc?.kill("SIGTERM");
    this.proc = undefined;
    this.updateStatus("stopped");
  }

  /**
   * Send one request and correlate its response by id.
   *
   * Each call is assigned a unique, monotonically increasing `id` (never reused
   * while a response is outstanding) and a `pending` entry is tracked for it.
   * Resolves with the response `result`, rejects with {@link StdioRequestError}
   * on an `error` response, or rejects with a plain `Error` if no response
   * arrives within `timeoutMs`. On timeout the pending entry is removed, so a
   * late-arriving response for that id is silently discarded by
   * {@link handleLine}.
   */
  async request<TParams, TResult>(
    method: string,
    params: TParams,
    timeoutMs: number = DEFAULT_REQUEST_TIMEOUT_MS
  ): Promise<TResult> {
    if (!this.isRunning || !this.proc?.stdin) {
      throw new Error("no running server; call ensureRunning()");
    }
    const id = this.nextId++;
    const line = JSON.stringify({ id, method, params }) + "\n";

    return new Promise<TResult>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(
          new Error(`stdio request '${method}' timed out after ${timeoutMs}ms`)
        );
      }, timeoutMs);

      this.pending.set(id, {
        resolve: (v) => {
          clearTimeout(timer);
          resolve(v as TResult);
        },
        reject: (e) => {
          clearTimeout(timer);
          reject(e);
        },
        method,
      });

      this.enqueueWrite(line);
    });
  }

  /**
   * Write a request line to the child's stdin, honoring stream backpressure.
   *
   * While the stream is happily accepting data (`writeQueue` empty and not
   * `draining`) the line is written directly. The moment `stdin.write()`
   * returns `false` we flip {@link draining} and every subsequent line is
   * buffered in {@link writeQueue} until the `'drain'` event fires and
   * {@link flushWriteQueue} drains it. Node serializes writes internally, so
   * frames never interleave; this only bounds how much we hand the stream at
   * once. Payloads here are small JSON control messages (large artifacts travel
   * by path), so the queue stays tiny in practice.
   */
  private enqueueWrite(line: string): void {
    const stdin = this.proc?.stdin;
    if (!stdin) {
      return;
    }
    if (this.draining || this.writeQueue.length > 0) {
      this.writeQueue.push(line);
      return;
    }
    const ok = stdin.write(line);
    if (!ok) {
      this.draining = true;
    }
  }

  /**
   * Flush queued lines after stdin signals `'drain'`, stopping again the moment
   * a write reports fresh backpressure so at most one write is ever in flight
   * awaiting the next `'drain'`.
   */
  private flushWriteQueue(): void {
    this.draining = false;
    const stdin = this.proc?.stdin;
    if (!stdin) {
      this.writeQueue = [];
      return;
    }
    while (this.writeQueue.length > 0 && !this.draining) {
      const next = this.writeQueue.shift() as string;
      const ok = stdin.write(next);
      if (!ok) {
        this.draining = true;
      }
    }
  }

  /**
   * Reject every in-flight request with `err`. The map is cleared before any
   * `reject` runs so that a reject callback which re-enters (e.g. issues a new
   * request) cannot observe a half-emptied `pending`, and each entry settles
   * exactly once.
   */
  private rejectAllPending(err: Error): void {
    const entries = Array.from(this.pending.values());
    this.pending.clear();
    for (const entry of entries) {
      entry.reject(err);
    }
  }

  /**
   * Handle child termination from an `'exit'` or `'error'` event: drop the
   * process handle, discard any queued writes, reject every still-pending
   * request, and return the status bar to `stopped`. Idempotent — a second
   * event finds an empty `pending` and no `proc`, so it is a no-op.
   */
  private handleTermination(err: Error): void {
    this.proc = undefined;
    this.writeQueue = [];
    this.draining = false;
    this.rejectAllPending(err);
    this.updateStatus("stopped");
  }

  /**
   * Buffer partial stdout chunks (Node delivers arbitrary chunk boundaries, not
   * line boundaries) and dispatch each complete `\n`-terminated line.
   */
  private handleStdoutChunk(chunk: Buffer): void {
    this.lineBuffer += chunk.toString("utf8");
    let idx: number;
    while ((idx = this.lineBuffer.indexOf("\n")) >= 0) {
      const line = this.lineBuffer.slice(0, idx);
      this.lineBuffer = this.lineBuffer.slice(idx + 1);
      if (line.trim() === "") {
        continue;
      }
      this.handleLine(line);
    }
  }

  /**
   * Parse one response line and settle its correlated pending entry. A line
   * that fails to parse is logged and dropped (it must never crash the
   * extension). A response whose `id` is missing/non-numeric, or that
   * references an id with no live pending entry — because it already settled,
   * timed out, or the request was disposed — is silently discarded so that
   * duplicate/late/unknown responses cannot double-settle a promise.
   */
  private handleLine(line: string): void {
    let msg: { id?: unknown; result?: unknown; error?: RpcError };
    try {
      msg = JSON.parse(line);
    } catch {
      this.output.appendLine(`[warn] dropping unparseable line: ${line}`);
      return;
    }
    if (typeof msg.id !== "number") {
      // Parse-error responses (id: null) and any malformed frame carry no id we
      // can correlate; nothing to settle.
      return;
    }
    const entry = this.pending.get(msg.id);
    if (!entry) {
      // Response for an id we no longer track: unknown, already-settled
      // (duplicate/late), timed-out, or disposed. Ignore it.
      return;
    }
    // Remove first so a duplicate response for the same id finds no entry.
    this.pending.delete(msg.id);
    if ("error" in msg && msg.error !== undefined) {
      entry.reject(
        new StdioRequestError(
          msg.error ?? { code: 0, message: "unknown error" }
        )
      );
    } else {
      entry.resolve(msg.result);
    }
  }

  private async resolveAssetRoot(): Promise<string | undefined> {
    const c = cfg();
    let configured = c.get<string>("assetRoot", "").trim();
    const wsFolder = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;

    if (!configured) {
      const picked = await vscode.window.showOpenDialog({
        canSelectFiles: false,
        canSelectFolders: true,
        canSelectMany: false,
        openLabel: "Use as asset root",
        title: "Select the robo-fitting-room asset root",
        defaultUri: wsFolder ? vscode.Uri.file(wsFolder) : undefined,
      });
      if (!picked || picked.length === 0) {
        return undefined;
      }
      configured = picked[0].fsPath;
      await c.update("assetRoot", configured, vscode.ConfigurationTarget.Workspace);
    }

    const resolved = path.isAbsolute(configured)
      ? configured
      : path.join(wsFolder ?? process.cwd(), configured);

    if (!fs.existsSync(resolved)) {
      vscode.window.showErrorMessage(`Asset root does not exist: ${resolved}`);
      return undefined;
    }
    return resolved;
  }

  dispose(): void {
    if (this.disposed) {
      return;
    }
    this.disposed = true;

    const proc = this.proc;
    if (proc) {
      // Drop our listeners first: the imminent SIGTERM will fire `'exit'`
      // asynchronously, and we don't want that callback touching the
      // already-disposed status bar / output channel.
      proc.removeAllListeners();
      proc.stdout?.removeAllListeners();
      proc.stderr?.removeAllListeners();
      proc.stdin?.removeAllListeners();
      if (proc.exitCode === null) {
        this.output.appendLine("Disposing: terminating managed process…");
        proc.kill("SIGTERM");
      }
    }
    this.proc = undefined;
    this.writeQueue = [];
    this.draining = false;

    // Reject anything still awaiting a response so no promise hangs forever.
    this.rejectAllPending(
      new StdioProcessError("StdioClient disposed before request completed")
    );

    this.status.dispose();
    this.output.dispose();
  }
}

function cfg(): vscode.WorkspaceConfiguration {
  return vscode.workspace.getConfiguration("roboFittingRoom");
}

function errMsg(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}
