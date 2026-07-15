/**
 * Example-based unit tests for the catalogue prompt-until-valid-or-cancel
 * loop (task 8.2).
 *
 * `resolveCatalogue()` (src/catalogue.ts) prompts via
 * `vscode.window.showOpenDialog` and validates the picked folder with
 * `fs.existsSync`, looping on an invalid selection and re-prompting until a
 * valid path is chosen or the user cancels (Req 5.5-5.7). Both `vscode` and
 * `fs` are module-level imports (not injected), so both are mocked here,
 * following the `vi.mock("vscode", ...)` convention already established by
 * `robo-fitting-room-vscode`'s `stdioClient.*.test.ts` files.
 *
 * Covers:
 *   - a valid path selected on the first try resolves immediately, with no
 *     error message shown (Req 5.5)
 *   - cancelling (no selection) resolves to `undefined` without re-prompting
 *     and without showing an error message (Req 5.6)
 *   - an invalid path followed by a cancel shows exactly one invalid-path
 *     error message before resolving to `undefined` (Req 5.6)
 *   - an invalid path followed by a valid path re-prompts exactly once, shows
 *     exactly one error message, and resolves to the valid path (Req 5.7)
 *   - two invalid paths in a row followed by a valid path show exactly two
 *     error messages (one per invalid attempt, not more/less) before
 *     resolving to the valid path (Req 5.7)
 *
 * _Requirements: 5.5, 5.6, 5.7_
 */

import { describe, it, expect, vi, beforeEach } from "vitest";

const showOpenDialog = vi.fn();
const showErrorMessage = vi.fn();

vi.mock("vscode", () => ({
  window: {
    showOpenDialog: (...args: unknown[]) => showOpenDialog(...args),
    showErrorMessage: (...args: unknown[]) => showErrorMessage(...args),
  },
  workspace: {
    workspaceFolders: undefined,
  },
  Uri: { file: (p: string) => ({ fsPath: p }) },
}));

const existsSync = vi.fn();

vi.mock("fs", () => ({
  existsSync: (...args: unknown[]) => existsSync(...args),
}));

import { resolveCatalogue } from "../../src/catalogue";

/** Wraps a picked path the way `showOpenDialog` resolves on a selection. */
function picked(fsPath: string) {
  return [{ fsPath }];
}

describe("resolveCatalogue (example)", () => {
  beforeEach(() => {
    showOpenDialog.mockReset();
    showErrorMessage.mockReset();
    existsSync.mockReset();
  });

  it("resolves to the path on a valid first selection, with no error shown (Req 5.5)", async () => {
    const validPath = "/catalogues/valid";
    showOpenDialog.mockResolvedValueOnce(picked(validPath));
    existsSync.mockImplementation((p: string) => p === validPath);

    const result = await resolveCatalogue();

    expect(result).toBe(validPath);
    expect(showOpenDialog).toHaveBeenCalledTimes(1);
    expect(showErrorMessage).not.toHaveBeenCalled();
  });

  it("resolves to undefined on cancel, without re-prompting or showing an error (Req 5.6)", async () => {
    showOpenDialog.mockResolvedValueOnce(undefined);

    const result = await resolveCatalogue();

    expect(result).toBeUndefined();
    expect(showOpenDialog).toHaveBeenCalledTimes(1);
    expect(showErrorMessage).not.toHaveBeenCalled();
  });

  it("resolves to undefined on cancel when showOpenDialog resolves to [] (Req 5.6)", async () => {
    showOpenDialog.mockResolvedValueOnce([]);

    const result = await resolveCatalogue();

    expect(result).toBeUndefined();
    expect(showOpenDialog).toHaveBeenCalledTimes(1);
    expect(showErrorMessage).not.toHaveBeenCalled();
  });

  it("shows exactly one error then resolves to undefined: invalid path then cancel (Req 5.6, 5.7)", async () => {
    const invalidPath = "/catalogues/missing";
    showOpenDialog
      .mockResolvedValueOnce(picked(invalidPath))
      .mockResolvedValueOnce(undefined);
    existsSync.mockReturnValue(false);

    const result = await resolveCatalogue();

    expect(result).toBeUndefined();
    expect(showOpenDialog).toHaveBeenCalledTimes(2);
    expect(showErrorMessage).toHaveBeenCalledTimes(1);
  });

  it("re-prompts once and resolves to the valid path: invalid then valid (Req 5.7)", async () => {
    const invalidPath = "/catalogues/missing";
    const validPath = "/catalogues/valid";
    showOpenDialog
      .mockResolvedValueOnce(picked(invalidPath))
      .mockResolvedValueOnce(picked(validPath));
    existsSync.mockImplementation((p: string) => p === validPath);

    const result = await resolveCatalogue();

    expect(result).toBe(validPath);
    expect(showOpenDialog).toHaveBeenCalledTimes(2);
    expect(showErrorMessage).toHaveBeenCalledTimes(1);
  });

  it("shows exactly one error per invalid attempt: two invalid then valid (Req 5.7)", async () => {
    const invalidPathA = "/catalogues/missing-a";
    const invalidPathB = "/catalogues/missing-b";
    const validPath = "/catalogues/valid";
    showOpenDialog
      .mockResolvedValueOnce(picked(invalidPathA))
      .mockResolvedValueOnce(picked(invalidPathB))
      .mockResolvedValueOnce(picked(validPath));
    existsSync.mockImplementation((p: string) => p === validPath);

    const result = await resolveCatalogue();

    expect(result).toBe(validPath);
    expect(showOpenDialog).toHaveBeenCalledTimes(3);
    expect(showErrorMessage).toHaveBeenCalledTimes(2);
  });
});
