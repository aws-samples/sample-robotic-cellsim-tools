import { EventEmitter } from "events";
import { describe, it, expect, vi } from "vitest";

// StdioClient does `import * as vscode from "vscode"`, but there is no runtime
// `vscode` module outside the extension host. Mock the small surface the
// constructor and dispose() touch so the client can be constructed in a plain
// Node/vitest environment.
vi.mock("vscode", () => {
  const makeOutputChannel = () => ({
    appendLine: vi.fn(),
    append: vi.fn(),
    show: vi.fn(),
    dispose: vi.fn(),
  });
  const makeStatusBarItem = () => ({
    text: "",
    tooltip: "",
    command: "",
    show: vi.fn(),
    dispose: vi.fn(),
  });
  return {
    StatusBarAlignment: { Left: 1, Right: 2 },
    ProgressLocation: { Notification: 15 },
    ConfigurationTarget: { Global: 1, Workspace: 2, WorkspaceFolder: 3 },
    window: {
      createOutputChannel: vi.fn(makeOutputChannel),
      createStatusBarItem: vi.fn(makeStatusBarItem),
      showErrorMessage: vi.fn(),
      showInformationMessage: vi.fn(),
      withProgress: vi.fn(),
    },
    workspace: {
      getConfiguration: vi.fn(() => ({
        get: (_key: string, dflt: unknown) => dflt,
        update: vi.fn(),
      })),
      workspaceFolders: undefined,
    },
  };
});

import { StdioClient, StdioProcessError } from "./stdioClient";

/**
 * A fake child process good enough for `StdioClient.request()` to see the
 * process as running (`exitCode === null`, a writable `stdin`) and for
 * `dispose()` to tear it down (`kill`, `removeAllListeners` on the process and
 * each stream). stdout/stderr are real EventEmitters so `removeAllListeners`
 * behaves.
 */
function makeFakeProc() {
  const stdin = Object.assign(new EventEmitter(), {
    write: vi.fn(() => true),
  });
  const stdout = new EventEmitter();
  const stderr = new EventEmitter();
  const proc = Object.assign(new EventEmitter(), {
    exitCode: null as number | null,
    stdin,
    stdout,
    stderr,
    kill: vi.fn(),
  });
  return proc;
}

describe("StdioClient.dispose()", () => {
  it("kills the child with SIGTERM and rejects every pending request", async () => {
    const client = new StdioClient();
    const proc = makeFakeProc();
    // Inject the fake child so request() sees isRunning === true.
    (client as unknown as { proc: unknown }).proc = proc;

    // Two requests that stay pending: no response is ever fed back.
    const p1 = client.request("assets.listRobots", {});
    const p2 = client.request("urdf.joints", { path: "robot.urdf" });

    // Both are tracked and were written to the child's stdin.
    const pending = (client as unknown as { pending: Map<number, unknown> })
      .pending;
    expect(pending.size).toBe(2);
    expect(proc.stdin.write).toHaveBeenCalledTimes(2);

    client.dispose();

    // The child was terminated.
    expect(proc.kill).toHaveBeenCalledWith("SIGTERM");

    // Every pending request rejected with a disposal error, and none linger.
    await expect(p1).rejects.toBeInstanceOf(StdioProcessError);
    await expect(p2).rejects.toBeInstanceOf(StdioProcessError);
    await expect(p1).rejects.toThrow(/disposed/i);
    await expect(p2).rejects.toThrow(/disposed/i);
    expect(pending.size).toBe(0);
  });

  it("terminates the child even when there are no pending requests", async () => {
    const client = new StdioClient();
    const proc = makeFakeProc();
    (client as unknown as { proc: unknown }).proc = proc;

    client.dispose();

    expect(proc.kill).toHaveBeenCalledWith("SIGTERM");
    expect(
      (client as unknown as { pending: Map<number, unknown> }).pending.size
    ).toBe(0);
  });

  it("does not kill an already-exited child but still clears pending", async () => {
    const client = new StdioClient();
    const proc = makeFakeProc();
    (client as unknown as { proc: unknown }).proc = proc;

    const p1 = client.request("assets.listRobots", {});

    // Simulate the child having already exited before dispose runs.
    proc.exitCode = 0;
    client.dispose();

    // exitCode !== null ⇒ no redundant SIGTERM, but the pending request must
    // still be rejected so no promise hangs forever.
    expect(proc.kill).not.toHaveBeenCalled();
    await expect(p1).rejects.toBeInstanceOf(StdioProcessError);
  });
});
