import * as vscode from "vscode";
import * as path from "path";
import * as fs from "fs";
import { spawn } from "child_process";
import { StdioClient, StdioRequestError, RpcError } from "./stdioClient";

/**
 * Result of the `compose.run` / `compose.urdfBundle` stdio methods. The zip
 * is written to a uniquely named temp file whose absolute path (`zipPath`) is
 * returned instead of inline bytes, along with a suggested `downloadName` and
 * its size in `bytes`. Ownership of the temp file transfers to the host, which
 * moves it to the user-selected destination.
 */
interface ComposeArtifact {
  zipPath: string;
  downloadName: string;
  bytes: number;
}

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
/**
 * Generic RPC passthrough from the webview's `bridge.rpc(method, params)`
 * helper. Forwarded to the managed stdio child process; the reply echoes
 * `requestId` with either a `result` or an `error`.
 */
interface RpcRequest {
  type: "rpc";
  requestId: number;
  method: string;
  params: unknown;
}

type InboundMessage =
  | ComposeRequest
  | OpenYamlRequest
  | PromptSaveRequest
  | ExportBundleRequest
  | RpcRequest;

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

  static createOrShow(context: vscode.ExtensionContext, stdio: StdioClient): void {
    const column = vscode.window.activeTextEditor?.viewColumn ?? vscode.ViewColumn.One;
    if (EditorPanel.current) {
      EditorPanel.current.panel.reveal(column);
      // The backend is running now (openEditor awaited ensureRunning); re-run
      // the palette load in case it failed when the panel first opened.
      EditorPanel.current.post({ type: "refreshPalette" });
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
    EditorPanel.current = new EditorPanel(panel, context, stdio);
  }

  private constructor(
    panel: vscode.WebviewPanel,
    private readonly context: vscode.ExtensionContext,
    private readonly stdio: StdioClient
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
      case "rpc":
        return this.handleRpc(message);
    }
  }

  /**
   * Forward a generic RPC call from the webview to the managed stdio child
   * process and post the outcome back, keyed by the same `requestId`. Success
   * posts `{ requestId, result }`; failure posts `{ requestId, error }` with
   * the `{code, message, data}` shape `bridge.rpc` expects (it resolves
   * `res.result` or throws on `res.error`). `StdioRequestError`'s `rpcError`
   * is forwarded verbatim; any other error is synthesized into an error object.
   */
  private async handleRpc(req: RpcRequest): Promise<void> {
    try {
      const result = await this.stdio.request(req.method, req.params);
      this.post({ type: "rpcResult", requestId: req.requestId, result });
    } catch (e) {
      this.post({
        type: "rpcResult",
        requestId: req.requestId,
        error: toRpcError(e),
      });
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
    // Temp artifacts written by the child process that must be cleaned up if
    // the flow is cancelled or fails before they reach their destination.
    const orphans: string[] = [];
    try {
      const usda = await vscode.window.withProgress(
        { location: vscode.ProgressLocation.Notification, title: "robo-fitting-room" },
        async (progress) => {
          progress.report({ message: "Composing USDA…" });
          return this.stdio.request<
            { manifestPath: string; fullRobots: boolean; fullTools: boolean },
            ComposeArtifact
          >("compose.run", {
            manifestPath: req.manifestPath,
            fullRobots: req.fullRobots,
            fullTools: req.fullTools,
          });
        }
      );
      orphans.push(usda.zipPath);

      const target = await vscode.window.showSaveDialog({
        saveLabel: "Save USDA zip",
        defaultUri: this.defaultDialogUri(req.downloadName || usda.downloadName),
        filters: { "Zip archive": ["zip"] },
      });
      if (!target) {
        this.replyCompose(req, { ok: false, message: "Compose cancelled." });
        return;
      }

      await moveFile(usda.zipPath, target.fsPath);
      removeItem(orphans, usda.zipPath);

      let savedBundle: string | undefined;
      if (req.urdfBundle) {
        const bundle = await vscode.window.withProgress(
          { location: vscode.ProgressLocation.Notification, title: "robo-fitting-room" },
          async (progress) => {
            progress.report({ message: "Bundling source URDFs…" });
            return this.stdio.request<{ manifestPath: string }, ComposeArtifact>(
              "compose.urdfBundle",
              { manifestPath: req.manifestPath }
            );
          }
        );
        orphans.push(bundle.zipPath);
        const bundleUri = vscode.Uri.file(
          path.join(path.dirname(target.fsPath), req.bundleName)
        );
        await moveFile(bundle.zipPath, bundleUri.fsPath);
        removeItem(orphans, bundle.zipPath);
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
    } catch (e) {
      const msg = `Compose failed: ${errMsg(e)}`;
      this.replyCompose(req, { ok: false, message: msg });
      vscode.window.showErrorMessage(msg);
    } finally {
      // Remove any temp artifacts we never moved to a destination.
      for (const p of orphans) {
        void fs.promises.unlink(p).catch(() => undefined);
      }
    }
  }

  private async handleExportBundle(req: ExportBundleRequest): Promise<void> {
    // Temp artifact written by the child process; cleaned up if we don't move
    // or extract it.
    let orphanZip: string | undefined;
    try {
      const bundle = await vscode.window.withProgress(
        { location: vscode.ProgressLocation.Notification, title: "robo-fitting-room" },
        async (progress) => {
          progress.report({ message: "Bundling source URDFs…" });
          return this.stdio.request<{ manifestPath: string }, ComposeArtifact>(
            "compose.urdfBundle",
            { manifestPath: req.manifestPath }
          );
        }
      );
      orphanZip = bundle.zipPath;

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
      const stem =
        path.basename(req.manifestPath).replace(/\.(ya?ml)$/i, "") ||
        req.manifestPath ||
        "scene";
      const destDir = path.join(picked[0].fsPath, `${stem}-urdf`);

      const written = await vscode.window.withProgress(
        { location: vscode.ProgressLocation.Notification, title: "robo-fitting-room" },
        async (progress) => {
          progress.report({ message: "Extracting…" });
          return this.writeBundle(bundle.zipPath, destDir);
        }
      );
      // writeBundle consumes (moves/deletes) the temp zip, so it's no longer
      // an orphan to clean up.
      orphanZip = undefined;
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
    } catch (e) {
      const msg = `Export failed: ${errMsg(e)}`;
      this.replyExport(req, { ok: false, message: msg });
      vscode.window.showErrorMessage(msg);
    } finally {
      if (orphanZip) {
        void fs.promises.unlink(orphanZip).catch(() => undefined);
      }
    }
  }

  private replyExport(req: ExportBundleRequest, result: { ok: boolean; message: string }): void {
    this.post({ type: "exportBundleResult", requestId: req.requestId, ...result });
  }

  /**
   * Extract a bundle temp zip (`srcZip`, produced by `compose.urdfBundle`)
   * into `destDir`. Falls back to moving the raw .zip next to the target when
   * `unzip` isn't available, so the user never loses the export. Either way the
   * temp zip is consumed (extracted then deleted, or moved), so the caller no
   * longer owns it after this returns.
   */
  private async writeBundle(
    srcZip: string,
    destDir: string
  ): Promise<{ message: string; revealPath: string }> {
    try {
      await fs.promises.mkdir(destDir, { recursive: true });
      await unzipTo(srcZip, destDir);
      return { message: `Exported URDF bundle → ${destDir}`, revealPath: destDir };
    } catch (e) {
      // Extraction unavailable/failed — preserve the archive instead by moving
      // the temp zip to its final resting place.
      const zipPath = `${destDir}.zip`;
      await moveFile(srcZip, zipPath);
      return {
        message: `Could not extract (${errMsg(e)}); saved archive → ${zipPath}`,
        revealPath: zipPath,
      };
    } finally {
      // On the extraction path the temp zip is no longer needed; on the
      // fallback path it has already been moved (unlink is then a no-op).
      void fs.promises.unlink(srcZip).catch(() => undefined);
    }
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

    const nonce = makeNonce();
    // The webview no longer issues cross-origin fetch()s — every backend call
    // is forwarded through the postMessage bridge to the stdio child process —
    // so no network origin needs to be allowed in connect-src.
    const csp = [
      `default-src 'none'`,
      `img-src ${webview.cspSource} data:`,
      `style-src ${webview.cspSource}`,
      `font-src ${webview.cspSource}`,
      `script-src ${webview.cspSource} 'nonce-${nonce}'`,
    ].join("; ");

    return /* html */ `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta http-equiv="Content-Security-Policy" content="${csp}" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>robo-fitting-room — scene editor</title>
    <link rel="stylesheet" href="${media("style.css")}" />
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

/**
 * Project any thrown value into the `{code, message, data}` error envelope the
 * webview's `bridge.rpc` reads off an `{error}` reply. A `StdioRequestError`
 * carries the backend's `rpcError` verbatim; anything else (process crash,
 * timeout, unexpected throw) is synthesized with a generic code and its
 * message.
 */
function toRpcError(e: unknown): RpcError {
  if (e instanceof StdioRequestError) {
    return e.rpcError;
  }
  return { code: 0, message: errMsg(e) };
}

/**
 * Move `src` to `dest`, preferring an atomic `fs.rename`. `rename` fails with
 * `EXDEV` when the two paths live on different filesystems (a temp file under
 * the OS temp dir being moved onto another volume is the common case here), so
 * on that error fall back to a copy followed by unlinking the source.
 */
export async function moveFile(src: string, dest: string): Promise<void> {
  try {
    await fs.promises.rename(src, dest);
  } catch (e) {
    if ((e as NodeJS.ErrnoException).code === "EXDEV") {
      await fs.promises.copyFile(src, dest);
      await fs.promises.unlink(src);
      return;
    }
    throw e;
  }
}

/** Remove the first occurrence of `value` from `arr`, in place. */
function removeItem<T>(arr: T[], value: T): void {
  const i = arr.indexOf(value);
  if (i >= 0) {
    arr.splice(i, 1);
  }
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
