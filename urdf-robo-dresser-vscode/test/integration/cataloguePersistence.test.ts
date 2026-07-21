/**
 * Integration test for persisting a catalogue folder picked via the
 * "prompt when unset" flow back to `roboDresser.catalogue` (see
 * `persistCatalogue()` in src/extension.ts).
 *
 * Before this behavior existed, `startAndOpenViewer()` held a picked
 * catalogue path only in a local variable for that one invocation —
 * `roboDresser.catalogue` stayed empty, so every subsequent launch
 * re-prompted. This test drives `roboDresser.startServer` end-to-end (via
 * the real `activate()` wiring) with the setting unset, confirms the picked
 * path is written back to `roboDresser.catalogue` exactly once, and confirms
 * a second launch with the setting now populated does not prompt again.
 *
 * `RoboDresserProcessManager` and `RoboDresserViewerPanel` are mocked out —
 * this test is about the config-persistence side effect, not process
 * spawning or webview rendering (already covered by
 * `processManager.test.ts` / `viewerPanel.test.ts` / `serverReady.test.ts`).
 */

import { describe, it, expect, vi, beforeEach } from "vitest";

const registerCommand = vi.fn();
const showOpenDialog = vi.fn();
const showErrorMessage = vi.fn();
const configUpdate = vi.fn();
const configGet = vi.fn();

vi.mock("vscode", () => ({
  commands: {
    registerCommand: (id: string, cb: () => unknown) => {
      registerCommand(id, cb);
      return { dispose: vi.fn() };
    },
  },
  workspace: {
    getConfiguration: (section: string) => ({
      get: (key: string, fallback?: unknown) => configGet(section, key, fallback),
      update: (key: string, value: unknown, target: unknown) =>
        configUpdate(section, key, value, target),
    }),
    workspaceFolders: undefined,
  },
  window: {
    showOpenDialog: (...args: unknown[]) => showOpenDialog(...args),
    showErrorMessage: (...args: unknown[]) => showErrorMessage(...args),
    showWarningMessage: vi.fn(),
    showInformationMessage: vi.fn(),
  },
  env: { openExternal: vi.fn() },
  Uri: { file: (p: string) => ({ fsPath: p }), parse: (s: string) => ({ toString: () => s }) },
  ConfigurationTarget: { Global: 1, Workspace: 2 },
}));

vi.mock("fs", () => ({
  existsSync: () => true,
}));

const ensureRunning = vi.fn().mockResolvedValue(true);
const baseUrl = vi.fn().mockReturnValue("http://127.0.0.1:3030");
const startReachabilityMonitor = vi.fn();

vi.mock("../../src/processManager", () => ({
  RoboDresserProcessManager: vi.fn().mockImplementation(() => ({
    ensureRunning,
    baseUrl,
    startReachabilityMonitor,
    stopReachabilityMonitor: vi.fn(),
    stop: vi.fn(),
    openDocs: vi.fn(),
  })),
}));

vi.mock("../../src/viewerPanel", () => ({
  RoboDresserViewerPanel: { createOrShow: vi.fn() },
}));

import { activate } from "../../src/extension";

describe("catalogue prompt result is persisted to roboDresser.catalogue (integration)", () => {
  beforeEach(() => {
    registerCommand.mockReset();
    showOpenDialog.mockReset();
    showErrorMessage.mockReset();
    configUpdate.mockReset();
    configGet.mockReset();
    ensureRunning.mockClear();
    baseUrl.mockClear();
    startReachabilityMonitor.mockClear();
  });

  it("writes the picked folder to roboDresser.catalogue exactly once when the setting starts empty", async () => {
    const pickedPath = "/catalogues/picked";
    configGet.mockImplementation((_section: string, key: string, fallback?: unknown) =>
      key === "catalogue" ? "" : fallback
    );
    showOpenDialog.mockResolvedValueOnce([{ fsPath: pickedPath }]);

    activate({ extensionPath: "/fake", subscriptions: [] } as never);
    const startServerCall = registerCommand.mock.calls.find(
      (call) => call[0] === "roboDresser.startServer"
    );
    expect(startServerCall).toBeDefined();
    await startServerCall![1]();

    expect(configUpdate).toHaveBeenCalledTimes(1);
    expect(configUpdate).toHaveBeenCalledWith(
      "roboDresser",
      "catalogue",
      pickedPath,
      /* ConfigurationTarget.Global, since no workspace folder is open */ 1
    );
    expect(ensureRunning).toHaveBeenCalledWith(
      expect.objectContaining({ catalogue: pickedPath }),
      expect.anything()
    );
  });

  it("does not prompt or write again once roboDresser.catalogue is already set", async () => {
    const existingPath = "/catalogues/already-set";
    configGet.mockImplementation((_section: string, key: string, fallback?: unknown) =>
      key === "catalogue" ? existingPath : fallback
    );

    activate({ extensionPath: "/fake", subscriptions: [] } as never);
    const startServerCall = registerCommand.mock.calls.find(
      (call) => call[0] === "roboDresser.startServer"
    );
    await startServerCall![1]();

    expect(showOpenDialog).not.toHaveBeenCalled();
    expect(configUpdate).not.toHaveBeenCalled();
    expect(ensureRunning).toHaveBeenCalledWith(
      expect.objectContaining({ catalogue: existingPath }),
      expect.anything()
    );
  });
});
