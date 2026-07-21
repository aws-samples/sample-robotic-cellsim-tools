/**
 * Extension entry point for `urdf-robo-dresser-vscode`. Wires the pure/impure
 * pieces built in tasks 1-8 (`RoboDresserProcessManager`, `resolveCatalogue`,
 * `RoboDresserViewerPanel`) into the four `roboDresser.*` commands, following
 * the "Runtime flow: launch" sequence diagram in the design doc and the same
 * real-dependency wiring convention as `robo-fitting-room-vscode`'s
 * `extension.ts`/`stdioClient.ts` (module-scoped manager constructed once in
 * `activate`, real `child_process.spawn`, real HTTP probing, real timers,
 * `vscode.window.show*Message` reporting).
 */

import * as vscode from "vscode";
import { spawn } from "child_process";
import * as http from "http";
import * as path from "path";
import * as fs from "fs";
import {
  RoboDresserProcessManager,
  RoboDresserStartOptions,
  ProcessSpawner,
  SpawnedProcess,
  HttpProbe,
  Clock,
  Reporter,
  StartFailureReason,
} from "./processManager";
import { resolveCatalogue } from "./catalogue";
import { RoboDresserViewerPanel } from "./viewerPanel";

/** Req 3.2/8.3: bound on how long a start attempt waits for `/readyz`. */
const START_TIMEOUT_MS = 30_000;
/** Req 8.4/8.5: reachability polling interval, capped at 10s. */
const REACHABILITY_INTERVAL_MS = 10_000;

/** The single manager instance for this extension host window, constructed
 * once in `activate` (mirrors `robo-fitting-room-vscode`'s module-scoped
 * `stdio` variable). */
let manager: RoboDresserProcessManager | undefined;

export function activate(context: vscode.ExtensionContext): void {
  manager = new RoboDresserProcessManager({
    spawner: new RealProcessSpawner(),
    probe: new RealHttpProbe(),
    clock: new RealClock(),
    reporter: new VsCodeReporter(),
    exists: fs.existsSync,
    bundledBinaryPath: bundledBinaryPath(context.extensionPath),
    openExternal: (url) => Promise.resolve(vscode.env.openExternal(vscode.Uri.parse(url))),
    host: "127.0.0.1",
  });

  context.subscriptions.push(
    vscode.commands.registerCommand("roboDresser.openViewer", () =>
      startAndOpenViewer(context)
    ),
    vscode.commands.registerCommand("roboDresser.startServer", () =>
      startAndOpenViewer(context)
    ),
    vscode.commands.registerCommand("roboDresser.stopServer", () => {
      manager!.stop();
      manager!.stopReachabilityMonitor();
    }),
    vscode.commands.registerCommand("roboDresser.openDocs", () => {
      manager!.openDocs();
    })
  );
}

export function deactivate(): void {
  // Req 3.7: stop the server this extension started on host deactivation.
  manager?.stop();
  manager = undefined;
}

/**
 * Shared flow for `roboDresser.openViewer` and `roboDresser.startServer`
 * (per the task's explicit instruction, both commands run the identical
 * catalogue-resolution + ensureRunning + viewer + reachability-monitor
 * flow): resolve the Catalogue directory when unset (Req 5.5), bail out
 * without starting anything if the user cancels that prompt (Req 5.6), then
 * hand off to `ensureRunning`. On success, open/reveal the viewer (Req
 * 3.2/3.4/4.3) and (re)start the reachability monitor (Req 8.4/8.5). On
 * failure, do nothing further — the manager's own `Reporter` has already
 * reported the specific failure reason (Req 3.5).
 */
async function startAndOpenViewer(context: vscode.ExtensionContext): Promise<void> {
  const config = vscode.workspace.getConfiguration("roboDresser");
  let catalogue = config.get<string>("catalogue", "").trim();
  const port = config.get<number>("port", 3030);
  const binaryPath = config.get<string>("binaryPath", "");
  const webRoot = config.get<string>("webRoot", "");

  if (!catalogue) {
    const picked = await resolveCatalogue();
    if (picked === undefined) {
      // Req 5.6: user cancelled — leave the server stopped and viewer closed.
      return;
    }
    catalogue = picked;
    // Persist the pick to `roboDresser.catalogue` (same setting the
    // Configuration sidebar's "Catalogue directory" field reads/writes —
    // see `directory-settings.ts` in the host extension) so this prompt does
    // not reappear on every launch; mirrors `roboticCellTools.cellDir`'s
    // already-persisted, prompt-free behavior once set.
    await persistCatalogue(catalogue);
  }

  const options: RoboDresserStartOptions = { catalogue, port, binaryPath, webRoot };
  const ready = await manager!.ensureRunning(options, START_TIMEOUT_MS);
  if (!ready) {
    return;
  }

  RoboDresserViewerPanel.createOrShow(context, manager!.baseUrl()!);
  manager!.startReachabilityMonitor(REACHABILITY_INTERVAL_MS);
}

/**
 * Persist `value` to `roboDresser.catalogue`, using the same
 * Workspace-if-a-workspace-folder-is-open-else-Global target rule already
 * used by the host extension's directory-setting pickers
 * (`DirectorySettingsController.pickValue` / `extension.ts`'s
 * `writeSetting`), so a folder picked here behaves identically to one picked
 * through the Configuration sidebar's "Catalogue directory" field.
 */
async function persistCatalogue(value: string): Promise<void> {
  const target = vscode.workspace.workspaceFolders?.length
    ? vscode.ConfigurationTarget.Workspace
    : vscode.ConfigurationTarget.Global;
  await vscode.workspace.getConfiguration("roboDresser").update("catalogue", value, target);
}

/**
 * Resolve the bundled `urdf-robo-dresser[.exe]` binary path shipped inside
 * this (architecture-specific) extension, mirroring
 * `stdioClient.ts`'s `bundledBinaryPath()` (per-platform exe suffix,
 * `<extensionPath>/bin/<name>`). Unlike that precedent, existence is not
 * checked here — passing a non-existent path through unconditionally lets
 * `RoboDresserProcessManager` surface a `spawn-error` if it's ever actually
 * used (i.e. `roboDresser.binaryPath` is unset), matching the design's
 * failure-reason table ("a missing bundled binary surfaces later as a
 * spawn-error, not an invalid-path").
 */
function bundledBinaryPath(extensionPath: string): string {
  const exe = process.platform === "win32" ? "urdf-robo-dresser.exe" : "urdf-robo-dresser";
  return path.join(extensionPath, "bin", exe);
}

/** Real `child_process.spawn`-backed {@link ProcessSpawner}. */
class RealProcessSpawner implements ProcessSpawner {
  spawn(binaryPath: string, args: readonly string[]): SpawnedProcess {
    const proc = spawn(binaryPath, args as string[]);
    return {
      kill: () => proc.kill(),
      onExit: (cb) => proc.on("exit", (code) => cb(code)),
    };
  }
}

/**
 * Real HTTP-backed {@link HttpProbe}. A probe, not a request: any error
 * (connection refused, DNS failure, timeout, non-2xx) resolves `false`
 * rather than rejecting, since a server that isn't reachable yet is an
 * expected, routine polling outcome.
 */
class RealHttpProbe implements HttpProbe {
  probe(baseUrl: string, path: "/readyz" | "/healthz"): Promise<boolean> {
    return new Promise<boolean>((resolve) => {
      const req = http.get(`${baseUrl}${path}`, (res) => {
        const status = res.statusCode ?? 0;
        res.resume(); // drain the response so the socket can be released
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

/** Real `setTimeout`/`setInterval`-backed {@link Clock}. */
class RealClock implements Clock {
  timeout(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms));
  }

  interval(ms: number, cb: () => void): { cancel(): void } {
    const handle = setInterval(cb, ms);
    return { cancel: () => clearInterval(handle) };
  }
}

/**
 * Real {@link Reporter} surfacing every distinct failure/status reason via a
 * `vscode.window.show*Message` call, per the design's Error Handling table:
 * a distinct, clear message per `StartFailureReason` kind, plus one each for
 * `reportBecameUnreachable`, `reportBecameReachableAgain`, `reportNoDocsUrl`,
 * and `reportDocsUnreachable`.
 */
class VsCodeReporter implements Reporter {
  reportStartFailure(reason: StartFailureReason): void {
    vscode.window.showErrorMessage(describeStartFailure(reason));
  }

  reportBecameUnreachable(): void {
    vscode.window.showWarningMessage(
      "Robo Dresser: the server has become unreachable. It may have crashed or been stopped externally."
    );
  }

  reportBecameReachableAgain(): void {
    vscode.window.showInformationMessage("Robo Dresser: the server is reachable again.");
  }

  reportNoDocsUrl(): void {
    vscode.window.showInformationMessage(
      "Robo Dresser: the server is not running, so its API documentation cannot be opened. Start the server first."
    );
  }

  reportDocsUnreachable(): void {
    vscode.window.showErrorMessage("Robo Dresser: could not open the API documentation.");
  }
}

function describeStartFailure(reason: StartFailureReason): string {
  switch (reason.kind) {
    case "invalid-port":
      return `Robo Dresser: the configured port (${reason.port}) is invalid. Set "roboDresser.port" to a value between 1 and 65535.`;
    case "invalid-path":
      return `Robo Dresser: "roboDresser.${reason.setting}" does not exist on disk: ${reason.path}`;
    case "spawn-error":
      return `Robo Dresser: failed to start the server process: ${reason.message}`;
    case "timeout":
      return `Robo Dresser: the server did not become ready within ${reason.timeoutMs}ms.`;
    default:
      return "Robo Dresser: the server failed to start.";
  }
}
