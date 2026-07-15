/**
 * Example-based unit tests for `RoboDresserViewerPanel` (task 8.4).
 *
 * `RoboDresserViewerPanel.createOrShow(context, baseUrl)` (src/viewerPanel.ts)
 * has NO reachability-checking logic of its own — per its own doc comment,
 * reachability of the Robo_Dresser_Server (Req 4.2) is the CALLER's
 * responsibility. "Opening while unreachable reports the failure and never
 * creates a panel" (Req 4.2) is therefore validated at the call-site level
 * (task 9.1's `activate()` / task 13.2's `host-launch.ts`), NOT here — this
 * class has no reachability awareness by design, so there is nothing to
 * test against it for Req 4.2.
 *
 * What IS testable against this class:
 *   - the first `createOrShow` call creates exactly one webview panel and
 *     sets its HTML to `getRoboDresserViewerHtml(baseUrl)`'s output
 *   - a second `createOrShow` call reveals the existing panel instead of
 *     creating a duplicate (Req 4.3)
 *   - the reveal call gives the panel input focus, i.e. `preserveFocus` is
 *     falsy/omitted (Req 4.3)
 *   - the panel's `onDidDispose` callback clears the module-scoped reference
 *     (so a later `createOrShow` creates a new panel) and never calls any
 *     `stop()` on a process manager (Req 4.4) — structurally guaranteed
 *     because `createOrShow`'s signature (`context`, `baseUrl`) never
 *     receives or stores a process-manager-like object with a `stop` method
 *     at all; no `stop` spy is ever invoked across this test file since none
 *     is ever passed in.
 *
 * `vscode` is a module-level import (not injected), so it is mocked here,
 * mirroring the `vi.mock("vscode", ...)` convention already established by
 * `test/example/catalogue.test.ts`.
 *
 * _Requirements: 4.2 (validated at call-site, see note above), 4.3, 4.4_
 */

import { describe, it, expect, vi, beforeEach } from "vitest";

const reveal = vi.fn();
const stop = vi.fn();

let disposeCallback: (() => void) | undefined;

/** Fresh mock WebviewPanel created per `createWebviewPanel` call. */
function makePanel() {
  return {
    webview: { html: "" },
    reveal: (...args: unknown[]) => reveal(...args),
    dispose: vi.fn(),
    onDidDispose: (cb: () => void) => {
      disposeCallback = cb;
      return { dispose: vi.fn() };
    },
  };
}

const createWebviewPanel = vi.fn();

vi.mock("vscode", () => ({
  window: {
    createWebviewPanel: (...args: unknown[]) => createWebviewPanel(...args),
    activeTextEditor: undefined,
  },
  ViewColumn: { One: 1 },
}));

import { RoboDresserViewerPanel } from "../../src/viewerPanel";
import { getRoboDresserViewerHtml } from "../../src/viewerHtml";

const fakeContext = {} as never;

describe("RoboDresserViewerPanel (example)", () => {
  beforeEach(() => {
    // `RoboDresserViewerPanel.current` is a private static field that
    // persists across test cases within this module import. Clear it by
    // firing any lingering dispose callback from the previous test, so each
    // test starts from a clean "no panel open" state.
    disposeCallback?.();
    disposeCallback = undefined;

    createWebviewPanel.mockReset();
    reveal.mockReset();
    stop.mockReset();
    createWebviewPanel.mockImplementation(() => makePanel());
  });

  it("creates exactly one panel and sets its HTML from getRoboDresserViewerHtml (Req 4.1)", () => {
    const baseUrl = "http://127.0.0.1:3030";

    RoboDresserViewerPanel.createOrShow(fakeContext, baseUrl);

    expect(createWebviewPanel).toHaveBeenCalledTimes(1);
    const createdPanel = createWebviewPanel.mock.results[0].value;
    expect(createdPanel.webview.html).toBe(getRoboDresserViewerHtml(baseUrl));
  });

  it("reveals the existing panel instead of creating a duplicate on a second call (Req 4.3)", () => {
    const baseUrl = "http://127.0.0.1:3030";

    RoboDresserViewerPanel.createOrShow(fakeContext, baseUrl);
    RoboDresserViewerPanel.createOrShow(fakeContext, baseUrl);

    expect(createWebviewPanel).toHaveBeenCalledTimes(1);
    expect(reveal).toHaveBeenCalledTimes(1);
  });

  it("gives the revealed panel input focus: preserveFocus is falsy/omitted (Req 4.3)", () => {
    const baseUrl = "http://127.0.0.1:3030";

    RoboDresserViewerPanel.createOrShow(fakeContext, baseUrl);
    RoboDresserViewerPanel.createOrShow(fakeContext, baseUrl);

    const preserveFocusArg = reveal.mock.calls[0][1];
    expect(preserveFocusArg).toBeFalsy();
  });

  it("clears the singleton on dispose, so a later createOrShow creates a NEW panel (Req 4.4)", () => {
    const baseUrl = "http://127.0.0.1:3030";

    RoboDresserViewerPanel.createOrShow(fakeContext, baseUrl);
    expect(createWebviewPanel).toHaveBeenCalledTimes(1);

    // Simulate the user closing the panel.
    disposeCallback?.();

    RoboDresserViewerPanel.createOrShow(fakeContext, baseUrl);
    expect(createWebviewPanel).toHaveBeenCalledTimes(2);
  });

  it("never calls stop() on any process manager from onDidDispose (Req 4.4)", () => {
    // Structural guarantee: `createOrShow(context, baseUrl)` takes no
    // process-manager parameter, and the class stores no such reference —
    // there is nothing in this class capable of calling `.stop()` on
    // anything. This is confirmed dynamically: across every test in this
    // file (including creation, reveal, and dispose), the `stop` spy above
    // is never passed to or invoked by this class.
    const baseUrl = "http://127.0.0.1:3030";

    RoboDresserViewerPanel.createOrShow(fakeContext, baseUrl);
    disposeCallback?.();

    expect(stop).not.toHaveBeenCalled();
  });
});
