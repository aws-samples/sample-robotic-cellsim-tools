/**
 * Integration test for wrapper activation and command wiring (task 9.2).
 *
 * Exercises `activate(context)` (src/extension.ts) end-to-end against a
 * minimal fake `vscode.ExtensionContext`, without spawning any real process
 * or making any real network call: `activate` constructs a real
 * `RoboDresserProcessManager` wired to real `child_process`/`http`/timer
 * dependencies, but none of those are ever invoked here because no command
 * callback is executed — this test only confirms `activate()` itself runs
 * without throwing and registers the right commands, mirroring the
 * `vi.mock("vscode", ...)` convention already established by
 * `test/example/catalogue.test.ts` and `test/example/viewerPanel.test.ts`,
 * broadened to cover the larger `vscode` API surface `extension.ts` touches
 * (`commands.registerCommand`, `workspace.getConfiguration`,
 * `window.show*Message`, `window.createWebviewPanel`,
 * `window.activeTextEditor`, `ViewColumn`, `window.showOpenDialog`,
 * `env.openExternal`, `Uri.parse`/`Uri.file`).
 *
 * Covers:
 *   - `activate(fakeContext)` does not throw
 *   - it registers exactly the four `roboDresser.*` commands
 *     (`openViewer`, `startServer`, `stopServer`, `openDocs`), in any order
 *   - all four returned disposables are pushed onto `context.subscriptions`
 *   - (complementary, static manifest check — not an `activate()` behavior:
 *     VS Code reads `contributes.configuration` from `package.json` itself;
 *     `activate()` never calls any API to "register" configuration
 *     properties at runtime) `package.json` declares the same four
 *     `roboDresser.*` configuration properties
 *
 * _Requirements: 2.2, 2.3_
 */

import * as fs from "fs";
import * as path from "path";

import { describe, it, expect, vi, beforeEach } from "vitest";

const registerCommand = vi.fn();
const showOpenDialog = vi.fn();
const showErrorMessage = vi.fn();
const showWarningMessage = vi.fn();
const showInformationMessage = vi.fn();
const createWebviewPanel = vi.fn();
const openExternal = vi.fn();
const getConfiguration = vi.fn();

vi.mock("vscode", () => ({
  commands: {
    registerCommand: (...args: unknown[]) => registerCommand(...args),
  },
  workspace: {
    getConfiguration: (...args: unknown[]) => getConfiguration(...args),
    workspaceFolders: undefined,
  },
  window: {
    showOpenDialog: (...args: unknown[]) => showOpenDialog(...args),
    showErrorMessage: (...args: unknown[]) => showErrorMessage(...args),
    showWarningMessage: (...args: unknown[]) => showWarningMessage(...args),
    showInformationMessage: (...args: unknown[]) => showInformationMessage(...args),
    createWebviewPanel: (...args: unknown[]) => createWebviewPanel(...args),
    activeTextEditor: undefined,
  },
  env: {
    openExternal: (...args: unknown[]) => openExternal(...args),
  },
  Uri: {
    file: (p: string) => ({ fsPath: p }),
    parse: (s: string) => ({ toString: () => s }),
  },
  ViewColumn: { One: 1 },
}));

import { activate } from "../../src/extension";

/** Minimal fake `vscode.ExtensionContext` — `activate()` only reads
 * `context.extensionPath` (to resolve the bundled binary path) and pushes
 * disposables onto `context.subscriptions`. */
function makeFakeContext(): { extensionPath: string; subscriptions: unknown[] } {
  return { extensionPath: "/fake/path", subscriptions: [] };
}

describe("activate() (integration)", () => {
  beforeEach(() => {
    registerCommand.mockReset();
    showOpenDialog.mockReset();
    showErrorMessage.mockReset();
    showWarningMessage.mockReset();
    showInformationMessage.mockReset();
    createWebviewPanel.mockReset();
    openExternal.mockReset();
    getConfiguration.mockReset();

    // Each registerCommand call returns a distinct fake disposable so we can
    // confirm all four (and only those four) are pushed onto subscriptions.
    let counter = 0;
    registerCommand.mockImplementation(() => ({ dispose: vi.fn(), id: counter++ }));
  });

  it("does not throw and registers exactly the four roboDresser.* commands (Req 2.2)", () => {
    const fakeContext = makeFakeContext();

    expect(() => activate(fakeContext as never)).not.toThrow();

    expect(registerCommand).toHaveBeenCalledTimes(4);
    const registeredIds = registerCommand.mock.calls.map((call) => call[0]);
    expect(new Set(registeredIds)).toEqual(
      new Set([
        "roboDresser.openViewer",
        "roboDresser.startServer",
        "roboDresser.stopServer",
        "roboDresser.openDocs",
      ])
    );
  });

  it("pushes all four registered disposables onto context.subscriptions", () => {
    const fakeContext = makeFakeContext();

    activate(fakeContext as never);

    const returnedDisposables = registerCommand.mock.results.map((result) => result.value);
    expect(fakeContext.subscriptions).toHaveLength(4);
    for (const disposable of returnedDisposables) {
      expect(fakeContext.subscriptions).toContain(disposable);
    }
  });
});

// Complementary static manifest check (Req 2.3): configuration properties are
// declared in package.json's `contributes.configuration` and read by VS Code
// itself — `activate()` never calls any API to "register" them at runtime,
// so this is necessarily a package.json check rather than an activate()
// behavior assertion.
describe("package.json configuration manifest (integration)", () => {
  it("declares the four roboDresser.* configuration properties (Req 2.3)", () => {
    const packageJsonPath = path.join(__dirname, "..", "..", "package.json");
    const packageJson = JSON.parse(fs.readFileSync(packageJsonPath, "utf8"));

    const keys = Object.keys(packageJson.contributes.configuration.properties);
    expect(new Set(keys)).toEqual(
      new Set([
        "roboDresser.catalogue",
        "roboDresser.port",
        "roboDresser.binaryPath",
        "roboDresser.webRoot",
      ])
    );
  });
});
