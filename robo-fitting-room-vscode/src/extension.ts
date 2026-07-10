import * as vscode from "vscode";
import { StdioClient } from "./stdioClient";
import { EditorPanel } from "./panel";

let stdio: StdioClient | undefined;

export function activate(context: vscode.ExtensionContext): void {
  stdio = new StdioClient(context.extensionPath);
  context.subscriptions.push(stdio);

  context.subscriptions.push(
    vscode.commands.registerCommand("roboFittingRoom.openEditor", async () => {
      const ok = await stdio!.ensureRunning();
      if (!ok) {
        vscode.window.showErrorMessage(
          "Could not start the robo-fitting-room process. See the " +
            '"robo-fitting-room" output channel for details.'
        );
        return;
      }
      // If a panel is already open (e.g. from an earlier attempt before the
      // asset root was set), createOrShow reveals it and refreshes its palette
      // now that the backend is running.
      EditorPanel.createOrShow(context, stdio!);
    }),

    vscode.commands.registerCommand("roboFittingRoom.startServer", async () => {
      await stdio!.start();
    }),

    vscode.commands.registerCommand("roboFittingRoom.stopServer", () => {
      stdio!.stop();
    }),

    vscode.commands.registerCommand("roboFittingRoom.openDocs", () => {
      // The docs live on a standalone HTTP `serve` instance, not the managed
      // stdio process. Resolve the URL from configuration directly.
      const serverUrl = vscode.workspace
        .getConfiguration("roboFittingRoom")
        .get<string>("serverUrl", "http://localhost:3040")
        .replace(/\/+$/, "");
      void vscode.env.openExternal(vscode.Uri.parse(`${serverUrl}/docs`));
    })
  );
}

export function deactivate(): void {
  // Disposal is handled via context.subscriptions (StdioClient.dispose()),
  // which VS Code invokes on deactivate.
  stdio = undefined;
}
