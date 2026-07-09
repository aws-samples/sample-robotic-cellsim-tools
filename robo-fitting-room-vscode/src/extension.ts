import * as vscode from "vscode";
import { ServerManager } from "./server";
import { EditorPanel } from "./panel";

let server: ServerManager | undefined;

export function activate(context: vscode.ExtensionContext): void {
  server = new ServerManager();
  context.subscriptions.push(server);

  context.subscriptions.push(
    vscode.commands.registerCommand("roboFittingRoom.openEditor", async () => {
      const ok = await server!.ensureRunning();
      if (!ok) {
        vscode.window.showWarningMessage(
          "Opening the editor without a reachable server — the palette and compose will fail until one is running."
        );
      }
      EditorPanel.createOrShow(context, server!);
    }),

    vscode.commands.registerCommand("roboFittingRoom.startServer", async () => {
      await server!.start();
    }),

    vscode.commands.registerCommand("roboFittingRoom.stopServer", () => {
      server!.stop();
    }),

    vscode.commands.registerCommand("roboFittingRoom.openDocs", () => {
      void vscode.env.openExternal(vscode.Uri.parse(`${server!.serverUrl}/docs`));
    })
  );
}

export function deactivate(): void {
  server?.dispose();
  server = undefined;
}
