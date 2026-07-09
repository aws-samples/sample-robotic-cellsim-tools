import * as vscode from "vscode";
import * as path from "path";
import * as fs from "fs";
import * as os from "os";
import { spawn } from "child_process";
import { ServerManager } from "./server";

/** Message shapes exchanged with the webview. */
interface ComposeRequest {
  type: "compose";
  requestId: number;
  manifestPath: string;
  fullRobots: boolean;
  fullTools: boolean;
  urdfBundle: boolean;
  downloadName: string;
  bundleName: string;
}
interface OpenYamlRequest {
  type: "openYaml";
  requestId: number;
}
interface PromptSaveRequest {
  type: "promptSaveName";
  requestId: number;
  current: string;
}
interface ExportBundleRequest {
  type: "exportBundle";
  requestId: number;
  manifestPath: string;
}

type InboundMessage =
  | ComposeRequest
  | OpenYamlRequest
  | PromptSaveRequest
  | ExportBundleRequest;

/**
 * Owns the single scene-editor webview panel and bridges its
 * non-fetch needs (native file dialogs, large zip downloads) to the
 * extension host.
 */
export class EditorPanel {
  public static current: EditorPanel | undefined;
  private static readonly viewType = "roboFittingRoom.editor";

  private readonly panel: vscode.WebviewPanel;
  private readonly disposables: vscode.Disposable[] = [];

  static createOrShow(context: vscode.ExtensionContext, server: ServerManager): void {
    const column = vscode.window.activeTextEditor?.viewColumn ?? vscode.ViewColumn.One;
    if (EditorPanel.current) {
      EditorPanel.current.panel.reveal(column);
      return;
    }
    const mediaRoot = vscode.Uri.joinPath(context.extensionUri, "media");
    const panel = vscode.window.createWebviewPanel(
      EditorPanel.viewType,
      "robo-fitting-room — Scene Editor",
      column,
      {
        enableScripts: true,
        retainContextWhenHidden: true,
        localResourceRoots: [mediaRoot],
      }
    );
    EditorPanel.current = new EditorPanel(panel, context, server);
  }

  private constructor(
    panel: vscode.WebviewPanel,
    private readonly context: vscode.ExtensionContext,
    private readonly server: ServerManager
  ) {
    this.panel = panel;
    this.panel.webview.html = this.buildHtml();
    this.panel.onDidDispose(() => this.dispose(), null, this.disposables);
    this.panel.webview.onDidReceiveMessage(
      (m: InboundMessage) => this.onMessage(m),
      null,
      this.disposables
    );
  }

  private post(message: unknown): void {
    void this.panel.webview.postMessage(message);
  }

  private async onMessage(message: InboundMessage): Promise<void> {
    switch (message.type) {
      case "openYaml":
        return this.handleOpenYaml(message);
      case "promptSaveName":
        return this.handlePromptSaveName(message);
      case "compose":
        return this.handleCompose(message);
      case "exportBundle":
        return this.handleExportBundle(message);
    }
  }

  private async handleOpenYaml(req: OpenYamlRequest): Promise<void> {
    const picked = await vscode.window.showOpenDialog({
      canSelectMany: false,
      openLabel: "Load Scene",
      filters: { "Scene manifest": ["yaml", "yml"] },
      defaultUri: this.defaultDialogUri(),
    });
    if (!picked || picked.length === 0) {
      this.post({ type: "openYamlResult", requestId: req.requestId, cancelled: true });
      return;
    }
    try {
      const bytes = await vscode.workspace.fs.readFile(picked[0]);
      this.post({
        type: "openYamlResult",
        requestId: req.requestId,
        cancelled: false,
        filename: path.basename(picked[0].fsPath),
        text: Buffer.from(bytes).toString("utf8"),
      });
    } catch (e) {
      this.post({ type: "openYamlResult", requestId: req.requestId, cancelled: true });
      vscode.window.showErrorMessage(`Failed to read file: ${errMsg(e)}`);
    }
  }

  private async handlePromptSaveName(req: PromptSaveRequest): Promise<void> {
    const name = await vscode.window.showInputBox({
      prompt: "Save manifest as (stored under fitted-robots/ on the server)",
      value: req.current,
      validateInput: (v) => (v.trim() ? undefined : "A filename is required"),
    });
    this.post({
      type: "promptSaveNameResult",
      requestId: req.requestId,
      name: name ?? null,
    });
  }

  private async handleCompose(req: ComposeRequest): Promise<void> {
    const base = this.server.serverUrl;
    try {
      // Pick the save destination up front so the user isn't surprised
      // after a long compose.
      const target = await vscode.window.showSaveDialog({
        saveLabel: "Save USDA zip",
        defaultUri: this.defaultDialogUri(req.downloadName),
        filters: { "Zip archive": ["zip"] },
      });
      if (!target) {
        this.replyCompose(req, { ok: false, message: "Compose cancelled." });
        return;
      }

      await vscode.window.withProgress(
        { location: vscode.ProgressLocation.Notification, title: "robo-fitting-room" },
        async (progress) => {
          progress.report({ message: "Composing USDA…" });
          const usdaBytes = await this.composeFetch(
            `${base}/api/v1/compose?` +
              new URLSearchParams({
                manifest_path: req.manifestPath,
                full_robots: String(req.fullRobots),
                full_tools: String(req.fullTools),
              })
          );
          await vscode.workspace.fs.writeFile(target, usdaBytes);

          let savedBundle: string | undefined;
          if (req.urdfBundle) {
            progress.report({ message: "Bundling source URDFs…" });
            const bundleBytes = await this.composeFetch(
              `${base}/api/v1/compose-urdf-bundle?` +
                new URLSearchParams({ manifest_path: req.manifestPath })
            );
            const bundleUri = vscode.Uri.file(
              path.join(path.dirname(target.fsPath), req.bundleName)
            );
            await vscode.workspace.fs.writeFile(bundleUri, bundleBytes);
            savedBundle = path.basename(bundleUri.fsPath);
          }

          const usdaName = path.basename(target.fsPath);
          const summary = savedBundle
            ? `Wrote ${usdaName} + ${savedBundle}`
            : `Wrote ${usdaName}`;
          this.replyCompose(req, { ok: true, message: summary });
          const open = await vscode.window.showInformationMessage(
            summary,
            "Reveal in Explorer"
          );
          if (open === "Reveal in Explorer") {
            void vscode.commands.executeCommand("revealFileInOS", target);
          }
        }
      );
    } catch (e) {
      const msg = `Compose failed: ${errMsg(e)}`;
      this.replyCompose(req, { ok: false, message: msg });
      vscode.window.showErrorMessage(msg);
    }
  }

  private async handleExportBundle(req: ExportBundleRequest): Promise<void> {
    const base = this.server.serverUrl;
    try {
      const picked = await vscode.window.showOpenDialog({
        canSelectFiles: false,
        canSelectFolders: true,
        canSelectMany: false,
        openLabel: "Export here",
        title: "Choose a folder to export the URDF bundle into",
        defaultUri: this.defaultDialogUri(),
      });
      if (!picked || picked.length === 0) {
        this.replyExport(req, { ok: false, message: "Export cancelled." });
        return;
      }

      // The bundle zip stores entries at its root (scene.yaml, robots/,
      // fixtures/, floorplans/), so extract into a named subfolder to keep
      // the tree self-contained.
      const stem = path.basename(req.manifestPath).replace(/\.(ya?ml)$/i, "") || "scene";
      const destDir = path.join(picked[0].fsPath, `${stem}-urdf`);

      await vscode.window.withProgress(
        { location: vscode.ProgressLocation.Notification, title: "robo-fitting-room" },
        async (progress) => {
          progress.report({ message: "Fetching URDF bundle…" });
          const bytes = await this.composeFetch(
            `${base}/api/v1/compose-urdf-bundle?` +
              new URLSearchParams({ manifest_path: req.manifestPath })
          );
          progress.report({ message: "Extracting…" });
          const written = await this.writeBundle(bytes, destDir);
          this.replyExport(req, { ok: true, message: written.message });
          const open = await vscode.window.showInformationMessage(
            written.message,
            "Reveal in Explorer"
          );
          if (open === "Reveal in Explorer") {
            void vscode.commands.executeCommand(
              "revealFileInOS",
              vscode.Uri.file(written.revealPath)
            );
          }
        }
      );
    } catch (e) {
      const msg = `Export failed: ${errMsg(e)}`;
      this.replyExport(req, { ok: false, message: msg });
      vscode.window.showErrorMessage(msg);
    }
  }

  private replyExport(req: ExportBundleRequest, result: { ok: boolean; message: string }): void {
    this.post({ type: "exportBundleResult", requestId: req.requestId, ...result });
  }

  /**
   * Write the bundle to disk as an extracted folder tree. Falls back to
   * saving the raw .zip next to the target when `unzip` isn't available,
   * so the user never loses the export.
   */
  private async writeBundle(
    bytes: Uint8Array,
    destDir: string
  ): Promise<{ message: string; revealPath: string }> {
    const tmpZip = path.join(os.tmpdir(), `rfr-bundle-${Date.now()}.zip`);
    await fs.promises.writeFile(tmpZip, bytes);
    try {
      await fs.promises.mkdir(destDir, { recursive: true });
      await unzipTo(tmpZip, destDir);
      return { message: `Exported URDF bundle → ${destDir}`, revealPath: destDir };
    } catch (e) {
      // Extraction unavailable/failed — preserve the archive instead.
      const zipPath = `${destDir}.zip`;
      await fs.promises.copyFile(tmpZip, zipPath);
      return {
        message: `Could not extract (${errMsg(e)}); saved archive → ${zipPath}`,
        revealPath: zipPath,
      };
    } finally {
      void fs.promises.unlink(tmpZip).catch(() => undefined);
    }
  }

  /** POST to a compose endpoint and return the response body as bytes. */
  private async composeFetch(url: string): Promise<Uint8Array> {
    const resp = await fetch(url, { method: "POST" });
    if (!resp.ok) {
      const body = await resp.text().catch(() => "");
      throw new Error(`${resp.status} ${resp.statusText}: ${body}`);
    }
    const buf = await resp.arrayBuffer();
    return new Uint8Array(buf);
  }

  private replyCompose(req: ComposeRequest, result: { ok: boolean; message: string }): void {
    this.post({ type: "composeResult", requestId: req.requestId, ...result });
  }

  private defaultDialogUri(filename?: string): vscode.Uri | undefined {
    const ws = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
    if (!ws) {
      return undefined;
    }
    return filename ? vscode.Uri.file(path.join(ws, filename)) : vscode.Uri.file(ws);
  }

  private buildHtml(): string {
    const webview = this.panel.webview;
    const media = (...p: string[]) =>
      webview.asWebviewUri(vscode.Uri.joinPath(this.context.extensionUri, "media", ...p));
    const jsYamlUri = media("vendor", "js-yaml.min.js");

    const serverUrl = this.server.serverUrl;
    const nonce = makeNonce();
    // connect-src must allow both http and ws forms of the server origin.
    const csp = [
      `default-src 'none'`,
      `img-src ${webview.cspSource} data:`,
      `style-src ${webview.cspSource}`,
      `font-src ${webview.cspSource}`,
      `script-src ${webview.cspSource} 'nonce-${nonce}'`,
      `connect-src ${serverUrl}`,
    ].join("; ");

    return /* html */ `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta http-equiv="Content-Security-Policy" content="${csp}" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>robo-fitting-room — scene editor</title>
    <link rel="stylesheet" href="${media("style.css")}" />
    <script nonce="${nonce}">
      window.__RFR_SERVER_URL__ = ${JSON.stringify(serverUrl)};
    </script>
    <script nonce="${nonce}" src="${jsYamlUri}"></script>
  </head>
  <body>
    <header>
      <strong>robo-fitting-room</strong>
      <span class="muted">scene editor</span>
      <span class="grow"></span>
      <button id="btn-load-yaml">Load Scene YAML…</button>
      <button id="btn-save">Save…</button>
      <button id="btn-export-bundle">Export Bundle…</button>
      <button id="btn-compose">Generate USDA</button>
    </header>

    <main>
      <aside id="palette">
        <button class="aside-toggle" data-aside="palette" title="Collapse palette">◂</button>
        <div class="aside-body">
          <h3>Palette</h3>
          <details open>
            <summary>Robots</summary>
            <ul id="palette-robots" class="palette-list"></ul>
          </details>
          <details open>
            <summary>Fixtures</summary>
            <ul id="palette-fixtures" class="palette-list"></ul>
          </details>
          <details>
            <summary>Floor plans</summary>
            <ul id="palette-floor-plans" class="palette-list"></ul>
          </details>
        </div>
        <div class="aside-collapsed-label">Palette</div>
      </aside>

      <section id="canvas-pane">
        <canvas id="canvas"></canvas>
        <div id="canvas-overlay">
          <div id="coord-readout" class="muted"></div>
          <div id="status-line">Ready.</div>
        </div>
      </section>

      <aside id="inspector">
        <button class="aside-toggle" data-aside="inspector" title="Collapse inspector">▸</button>
        <div class="aside-body">
          <h3>Inspector</h3>
          <div id="inspector-empty" class="muted">Select a robot or fixture to edit.</div>
          <div id="inspector-form" hidden></div>
        </div>
        <div class="aside-collapsed-label">Inspector</div>
      </aside>
    </main>

    <div id="compose-modal" class="modal" hidden>
      <div class="modal-backdrop"></div>
      <div class="modal-panel">
        <h3>Generate USDA</h3>

        <div class="form-group">
          <h4>Robot fidelity</h4>
          <label class="row-check">
            <input type="checkbox" id="opt-full-robots"/>
            <span>Full robot visuals <em>(off = collision hulls only — leaner zip)</em></span>
          </label>
        </div>

        <div class="form-group">
          <h4>Tool fidelity</h4>
          <label class="row-check">
            <input type="checkbox" id="opt-full-tools" checked/>
            <span>Full tool visuals <em>(off = collision hulls only)</em></span>
          </label>
        </div>

        <div class="form-group">
          <h4>URDF source bundle</h4>
          <label class="row-check">
            <input type="checkbox" id="opt-urdf-bundle"/>
            <span>Also save URDF source bundle <em>(second zip, self-contained)</em></span>
          </label>
        </div>

        <div class="form-group">
          <h4>Download</h4>
          <div class="row">
            <label>filename</label>
            <input type="text" id="opt-filename" value=""/>
          </div>
          <div class="muted" style="margin-top:4px;">
            Default: <code>scene-&lt;yyyymmddhhmmss&gt;.zip</code> (bundle: <code>&lt;name&gt;-urdf.zip</code>)
          </div>
        </div>

        <div class="btn-row">
          <button id="btn-compose-cancel">Cancel</button>
          <button id="btn-compose-go">Generate</button>
        </div>
      </div>
    </div>

    <script type="module" nonce="${nonce}" src="${media("js", "app.js")}"></script>
  </body>
</html>`;
  }

  dispose(): void {
    EditorPanel.current = undefined;
    this.panel.dispose();
    while (this.disposables.length) {
      this.disposables.pop()?.dispose();
    }
  }
}

function makeNonce(): string {
  const chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
  let out = "";
  for (let i = 0; i < 32; i++) {
    out += chars.charAt(Math.floor(Math.random() * chars.length));
  }
  return out;
}

function errMsg(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** Extract `zipPath` into `destDir` using the system `unzip`. Rejects if
 *  `unzip` is missing (ENOENT) or exits non-zero. */
function unzipTo(zipPath: string, destDir: string): Promise<void> {
  return new Promise((resolve, reject) => {
    const proc = spawn("unzip", ["-o", zipPath, "-d", destDir]);
    let stderr = "";
    proc.stderr.on("data", (d) => (stderr += d.toString()));
    proc.on("error", (e) =>
      reject(new Error(e.message.includes("ENOENT") ? "'unzip' not found on PATH" : e.message))
    );
    proc.on("exit", (code) =>
      code === 0 ? resolve() : reject(new Error(`unzip exited ${code}: ${stderr.trim()}`))
    );
  });
}
