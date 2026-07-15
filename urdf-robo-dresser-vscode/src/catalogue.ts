/**
 * Catalogue directory prompt (Req 5.5-5.7).
 *
 * Following the `resolveAssetRoot`/`showOpenDialog` pattern already used by
 * `robo-fitting-room-vscode`'s `stdioClient.ts`: prompt the User for a folder
 * via `vscode.window.showOpenDialog`, and loop on an invalid selection instead
 * of giving up after one attempt (that precedent shows an error and returns
 * `undefined` once; Req 5.6/5.7 for the Robo Dresser explicitly require
 * looping until a valid path is chosen or the User cancels).
 */

import * as vscode from "vscode";
import * as fs from "fs";

/**
 * Req 5.5-5.7: prompt-until-valid-or-cancel for the Catalogue directory.
 *
 * Loops:
 *   1. Show a folder-only open dialog.
 *   2. If the User cancels (no selection), return `undefined` immediately
 *      (Req 5.6) — the caller leaves the server stopped and the viewer
 *      closed, and this function does not re-prompt after a cancel.
 *   3. If a folder was selected but does not exist on disk, show a visible
 *      error message and loop back to (1) to re-prompt (Req 5.7).
 *   4. If the folder exists, return its path.
 */
export async function resolveCatalogue(): Promise<string | undefined> {
  const wsFolder = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;

  while (true) {
    const picked = await vscode.window.showOpenDialog({
      canSelectFiles: false,
      canSelectFolders: true,
      canSelectMany: false,
      openLabel: "Use as Catalogue directory",
      title: "Select the Robo Dresser Catalogue directory",
      defaultUri: wsFolder ? vscode.Uri.file(wsFolder) : undefined,
    });

    if (!picked || picked.length === 0) {
      // User cancelled: stop looping and leave the server stopped / viewer
      // closed (Req 5.6).
      return undefined;
    }

    const catalogue = picked[0].fsPath;
    if (!fs.existsSync(catalogue)) {
      vscode.window.showErrorMessage(`Catalogue directory does not exist: ${catalogue}`);
      continue;
    }

    return catalogue;
  }
}
