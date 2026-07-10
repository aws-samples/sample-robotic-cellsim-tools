import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import { afterEach, describe, it, expect, vi } from "vitest";

// panel.ts does `import * as vscode from "vscode"`, which has no runtime module
// outside the extension host. Importing `moveFile` pulls the whole module and
// triggers that import, so mock the small surface the module references at load
// time (nothing in moveFile touches it, but the top-level import must resolve).
vi.mock("vscode", () => ({
  StatusBarAlignment: { Left: 1, Right: 2 },
  ProgressLocation: { Notification: 15 },
  ViewColumn: { One: 1 },
  Uri: { file: (p: string) => ({ fsPath: p }), joinPath: (...p: unknown[]) => ({ fsPath: p.join("/") }) },
  window: {
    createWebviewPanel: vi.fn(),
    showOpenDialog: vi.fn(),
    showSaveDialog: vi.fn(),
    showInputBox: vi.fn(),
    showErrorMessage: vi.fn(),
    showInformationMessage: vi.fn(),
    withProgress: vi.fn(),
    activeTextEditor: undefined,
  },
  workspace: { workspaceFolders: undefined, fs: { readFile: vi.fn() } },
  commands: { executeCommand: vi.fn() },
}));

import { moveFile } from "./panel";

/** Create a fresh temp directory for a test and return its path. */
function makeTmpDir(): string {
  return fs.mkdtempSync(path.join(os.tmpdir(), "rfr-movefile-"));
}

describe("moveFile", () => {
  const cleanupDirs: string[] = [];

  afterEach(() => {
    vi.restoreAllMocks();
    for (const d of cleanupDirs.splice(0)) {
      fs.rmSync(d, { recursive: true, force: true });
    }
  });

  it("moves a file via rename when source and dest are on the same filesystem", async () => {
    const dir = makeTmpDir();
    cleanupDirs.push(dir);
    const src = path.join(dir, "source.zip");
    const dest = path.join(dir, "dest.zip");
    const contents = "hello-artifact-bytes";
    fs.writeFileSync(src, contents);

    await moveFile(src, dest);

    expect(fs.existsSync(dest)).toBe(true);
    expect(fs.readFileSync(dest, "utf8")).toBe(contents);
    expect(fs.existsSync(src)).toBe(false);
  });

  it("falls back to copy+unlink when rename fails with EXDEV", async () => {
    const dir = makeTmpDir();
    cleanupDirs.push(dir);
    const src = path.join(dir, "source.zip");
    const dest = path.join(dir, "dest.zip");
    const contents = "cross-device-artifact";
    fs.writeFileSync(src, contents);

    // Force the cross-device error on the first (and only) rename attempt.
    const renameSpy = vi
      .spyOn(fs.promises, "rename")
      .mockRejectedValueOnce(
        Object.assign(new Error("cross-device link not permitted"), { code: "EXDEV" })
      );
    const copySpy = vi.spyOn(fs.promises, "copyFile");
    const unlinkSpy = vi.spyOn(fs.promises, "unlink");

    await moveFile(src, dest);

    // Rename was attempted, then the copy+unlink fallback executed with the
    // right arguments.
    expect(renameSpy).toHaveBeenCalledTimes(1);
    expect(copySpy).toHaveBeenCalledWith(src, dest);
    expect(unlinkSpy).toHaveBeenCalledWith(src);

    // And the fallback actually produced the destination and removed the source
    // (copyFile/unlink ran against the real temp files).
    expect(fs.existsSync(dest)).toBe(true);
    expect(fs.readFileSync(dest, "utf8")).toBe(contents);
    expect(fs.existsSync(src)).toBe(false);
  });

  it("propagates a non-EXDEV rename error without attempting copy/unlink", async () => {
    const dir = makeTmpDir();
    cleanupDirs.push(dir);
    const src = path.join(dir, "source.zip");
    const dest = path.join(dir, "dest.zip");
    fs.writeFileSync(src, "data");

    vi.spyOn(fs.promises, "rename").mockRejectedValueOnce(
      Object.assign(new Error("permission denied"), { code: "EACCES" })
    );
    const copySpy = vi.spyOn(fs.promises, "copyFile");
    const unlinkSpy = vi.spyOn(fs.promises, "unlink");

    await expect(moveFile(src, dest)).rejects.toMatchObject({ code: "EACCES" });

    expect(copySpy).not.toHaveBeenCalled();
    expect(unlinkSpy).not.toHaveBeenCalled();
  });
});
