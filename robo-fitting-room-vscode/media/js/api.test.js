// Unit tests for media/js/api.js.
//
// These assert two things per export (Requirements 7.1, 7.2):
//   1. The wrapper keeps its exact public name and parameter signature
//      (arity), so app.js needs no changes.
//   2. The wrapper delegates to `bridge.rpc(method, params)` with the
//      exact method string and params object shape — no `fetch()`.
//
// `./bridge.js` calls `acquireVsCodeApi()` at module load, which only
// exists inside a real webview, so we mock the whole module and assert
// against the mocked `rpc`.

import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("./bridge.js", () => ({
  rpc: vi.fn(),
}));

import * as bridge from "./bridge.js";
import * as api from "./api.js";

beforeEach(() => {
  bridge.rpc.mockReset();
  // Default: every rpc call resolves to a benign value. Individual tests
  // override this when they need a specific shape (e.g. loadScene).
  bridge.rpc.mockResolvedValue(undefined);
});

describe("api.js export names and signatures are unchanged", () => {
  // [exportName, expectedArity]
  const expected = [
    ["listRobots", 0],
    ["listFixtures", 0],
    ["listFloorPlans", 0],
    ["urdfJoints", 1],
    ["floorPlanSegments", 1],
    ["validateYaml", 1],
    ["saveScene", 2],
    ["loadScene", 1],
    ["uploadObj", 2],
  ];

  it("exports exactly the expected set of functions", () => {
    const exportedFns = Object.keys(api).filter(
      (k) => typeof api[k] === "function"
    );
    expect(exportedFns.sort()).toEqual(expected.map(([n]) => n).sort());
  });

  for (const [name, arity] of expected) {
    it(`${name} exists as a function with arity ${arity}`, () => {
      expect(typeof api[name]).toBe("function");
      expect(api[name].name).toBe(name);
      expect(api[name].length).toBe(arity);
    });
  }
});

describe("api.js delegates to bridge.rpc with the correct method + params", () => {
  it("listRobots -> assets.listRobots with {}", async () => {
    await api.listRobots();
    expect(bridge.rpc).toHaveBeenCalledTimes(1);
    expect(bridge.rpc).toHaveBeenCalledWith("assets.listRobots", {});
  });

  it("listFixtures -> assets.listFixtures with {}", async () => {
    await api.listFixtures();
    expect(bridge.rpc).toHaveBeenCalledTimes(1);
    expect(bridge.rpc).toHaveBeenCalledWith("assets.listFixtures", {});
  });

  it("listFloorPlans -> assets.listFloorPlans with {}", async () => {
    await api.listFloorPlans();
    expect(bridge.rpc).toHaveBeenCalledTimes(1);
    expect(bridge.rpc).toHaveBeenCalledWith("assets.listFloorPlans", {});
  });

  it("urdfJoints(path) -> urdf.joints with { path }", async () => {
    await api.urdfJoints("robots/kuka.urdf");
    expect(bridge.rpc).toHaveBeenCalledTimes(1);
    expect(bridge.rpc).toHaveBeenCalledWith("urdf.joints", {
      path: "robots/kuka.urdf",
    });
  });

  it("floorPlanSegments(path) -> floorPlan.segments with { path }", async () => {
    await api.floorPlanSegments("floorplans/cell.obj");
    expect(bridge.rpc).toHaveBeenCalledTimes(1);
    expect(bridge.rpc).toHaveBeenCalledWith("floorPlan.segments", {
      path: "floorplans/cell.obj",
    });
  });

  it("validateYaml(yamlText) -> compose.validate with { yaml }", async () => {
    await api.validateYaml("scene: {}\n");
    expect(bridge.rpc).toHaveBeenCalledTimes(1);
    expect(bridge.rpc).toHaveBeenCalledWith("compose.validate", {
      yaml: "scene: {}\n",
    });
  });

  it("saveScene(path, yamlText) -> scene.save with { path, yaml }", async () => {
    await api.saveScene("scene.yaml", "scene: {}\n");
    expect(bridge.rpc).toHaveBeenCalledTimes(1);
    expect(bridge.rpc).toHaveBeenCalledWith("scene.save", {
      path: "scene.yaml",
      yaml: "scene: {}\n",
    });
  });

  it("loadScene(path) -> scene.load with { path } and unwraps res.yaml", async () => {
    bridge.rpc.mockResolvedValue({ yaml: "loaded: true\n" });
    const result = await api.loadScene("scene.yaml");
    expect(bridge.rpc).toHaveBeenCalledTimes(1);
    expect(bridge.rpc).toHaveBeenCalledWith("scene.load", {
      path: "scene.yaml",
    });
    expect(result).toBe("loaded: true\n");
  });
});

describe("uploadObj base64-encodes the file and delegates correctly", () => {
  class FakeFileReader {
    readAsDataURL(file) {
      const b64 = Buffer.from(file.content).toString("base64");
      this.result = `data:${file.type || "application/octet-stream"};base64,${b64}`;
      if (this.onload) this.onload();
    }
  }

  beforeEach(() => {
    vi.stubGlobal("FileReader", FakeFileReader);
  });

  it("uploadObj(file, filename) -> assets.uploadFloorPlanObj with { filename, dataBase64 }", async () => {
    const content = "o cube\nv 0 0 0\n";
    const expectedBase64 = Buffer.from(content).toString("base64");
    const file = { content, type: "model/obj" };

    await api.uploadObj(file, "cell.obj");

    expect(bridge.rpc).toHaveBeenCalledTimes(1);
    expect(bridge.rpc).toHaveBeenCalledWith("assets.uploadFloorPlanObj", {
      filename: "cell.obj",
      dataBase64: expectedBase64,
    });
  });

  it("uploadObj strips the data-URL prefix, sending only the base64 payload", async () => {
    const file = { content: "hello", type: "text/plain" };
    await api.uploadObj(file, "x.obj");

    const [, params] = bridge.rpc.mock.calls[0];
    expect(params.dataBase64).toBe(Buffer.from("hello").toString("base64"));
    expect(params.dataBase64).not.toContain("data:");
    expect(params.dataBase64).not.toContain(",");
  });
});
