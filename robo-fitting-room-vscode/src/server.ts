import * as vscode from "vscode";
import { ChildProcess, spawn } from "child_process";
import * as path from "path";
import * as fs from "fs";

/**
 * Manages the lifecycle of a `robo-fitting-room serve` child process and
 * exposes health/readiness checks against a (managed or external) server.
 */
export class ServerManager implements vscode.Disposable {
  private proc: ChildProcess | undefined;
  private readonly output: vscode.OutputChannel;
  private readonly status: vscode.StatusBarItem;

  constructor() {
    this.output = vscode.window.createOutputChannel("robo-fitting-room");
    this.status = vscode.window.createStatusBarItem(
      vscode.StatusBarAlignment.Left,
      100
    );
    this.status.command = "roboFittingRoom.openEditor";
    this.updateStatus("stopped");
  }

  /** Base URL of the configured server. */
  get serverUrl(): string {
    return cfg().get<string>("serverUrl", "http://localhost:3040").replace(/\/+$/, "");
  }

  private updateStatus(state: "running" | "stopped" | "external"): void {
    if (state === "running") {
      this.status.text = "$(vm-active) robo-fitting-room";
      this.status.tooltip = `Managed server running at ${this.serverUrl}`;
    } else if (state === "external") {
      this.status.text = "$(vm) robo-fitting-room";
      this.status.tooltip = `Reachable at ${this.serverUrl}`;
    } else {
      this.status.text = "$(vm-outline) robo-fitting-room";
      this.status.tooltip = "Server not running — click to open the editor";
    }
    this.status.show();
  }

  isManaged(): boolean {
    return this.proc !== undefined && this.proc.exitCode === null;
  }

  /** GET /healthz with a short timeout. Returns true when the server answers 200. */
  async isHealthy(): Promise<boolean> {
    try {
      const ctrl = new AbortController();
      const t = setTimeout(() => ctrl.abort(), 1500);
      const resp = await fetch(`${this.serverUrl}/healthz`, { signal: ctrl.signal });
      clearTimeout(t);
      const ok = resp.ok;
      if (ok) {
        this.updateStatus(this.isManaged() ? "running" : "external");
      }
      return ok;
    } catch {
      return false;
    }
  }

  /**
   * Ensure a server is reachable. If one is already answering, do nothing.
   * Otherwise offer to start the managed binary. Returns true if a server
   * is reachable by the time this resolves.
   */
  async ensureRunning(): Promise<boolean> {
    if (await this.isHealthy()) {
      return true;
    }
    const choice = await vscode.window.showInformationMessage(
      `No robo-fitting-room server reachable at ${this.serverUrl}.`,
      "Start Server",
      "Cancel"
    );
    if (choice !== "Start Server") {
      return false;
    }
    return this.start();
  }

  /** Spawn the managed `serve` process and wait until it reports healthy. */
  async start(): Promise<boolean> {
    if (this.isManaged()) {
      this.output.appendLine("Server already managed by this window.");
      return this.isHealthy();
    }
    if (await this.isHealthy()) {
      vscode.window.showInformationMessage(
        "A robo-fitting-room server is already reachable; not starting another."
      );
      return true;
    }

    const assetRoot = await this.resolveAssetRoot();
    if (!assetRoot) {
      return false;
    }

    const c = cfg();
    const bin = c.get<string>("binaryPath", "robo-fitting-room");
    const host = c.get<string>("serverHost", "127.0.0.1");
    const port = c.get<number>("serverPort", 3040);
    const webRoot = c.get<string>("webRoot", "").trim();

    const args = [
      "serve",
      "--host",
      host,
      "--port",
      String(port),
      "--asset-root",
      assetRoot,
    ];
    if (webRoot) {
      args.push("--web-root", webRoot);
    }

    this.output.show(true);
    this.output.appendLine(`$ ${bin} ${args.join(" ")}`);

    try {
      this.proc = spawn(bin, args, { cwd: assetRoot });
    } catch (e) {
      vscode.window.showErrorMessage(`Failed to spawn server: ${errMsg(e)}`);
      return false;
    }

    this.proc.stdout?.on("data", (d) => this.output.append(d.toString()));
    this.proc.stderr?.on("data", (d) => this.output.append(d.toString()));
    this.proc.on("error", (e) => {
      this.output.appendLine(`\n[error] ${e.message}`);
      vscode.window.showErrorMessage(
        `robo-fitting-room server error: ${e.message}. Check "roboFittingRoom.binaryPath".`
      );
    });
    this.proc.on("exit", (code, signal) => {
      this.output.appendLine(`\n[server exited] code=${code} signal=${signal}`);
      this.proc = undefined;
      this.updateStatus("stopped");
    });

    // Poll for readiness for up to ~10s.
    const ready = await vscode.window.withProgress(
      { location: vscode.ProgressLocation.Notification, title: "Starting robo-fitting-room server…" },
      async () => {
        for (let i = 0; i < 40; i++) {
          if (!this.isManaged()) {
            return false; // crashed during startup
          }
          if (await this.isHealthy()) {
            return true;
          }
          await delay(250);
        }
        return false;
      }
    );

    if (ready) {
      this.updateStatus("running");
      this.output.appendLine(`Server healthy at ${this.serverUrl}`);
    } else {
      vscode.window.showErrorMessage(
        "Server did not become healthy in time. See the robo-fitting-room output channel."
      );
    }
    return ready;
  }

  /** Terminate the managed process (no-op if the server is external). */
  stop(): void {
    if (!this.isManaged()) {
      vscode.window.showInformationMessage(
        "No managed server to stop (an external server, if any, is left alone)."
      );
      return;
    }
    this.output.appendLine("Stopping managed server…");
    this.proc?.kill("SIGINT");
    this.proc = undefined;
    this.updateStatus("stopped");
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
    this.stop();
    this.status.dispose();
    this.output.dispose();
  }
}

function cfg(): vscode.WorkspaceConfiguration {
  return vscode.workspace.getConfiguration("roboFittingRoom");
}

function delay(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

function errMsg(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}
