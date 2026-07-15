/**
 * Example-based unit tests for the `urdf-robo-dresser-vscode` package manifest
 * (task 1.2).
 *
 * Pins down the facts the rest of the wrapper package (and the runtime host's
 * manifest discovery/aggregation pipeline) rely on:
 *
 *   - every contributed command id starts with `roboDresser.` (Req 2.2)
 *   - every contributed configuration key starts with `roboDresser.` (Req 2.3)
 *   - `roboDresser.port`'s schema declares `minimum: 1, maximum: 65535` (Req 5.2)
 *
 * _Requirements: 2.2, 2.3, 5.2_
 */

import * as fs from "fs";
import * as path from "path";

import { describe, it, expect } from "vitest";

const packageJsonPath = path.join(__dirname, "..", "..", "package.json");
const packageJson = JSON.parse(fs.readFileSync(packageJsonPath, "utf8"));

describe("package.json manifest (example)", () => {
  it("gives every contributed command id a `roboDresser.` prefix (Req 2.2)", () => {
    const commands = packageJson.contributes.commands;
    expect(Array.isArray(commands)).toBe(true);
    expect(commands.length).toBeGreaterThan(0);
    for (const command of commands) {
      expect(command.command.startsWith("roboDresser.")).toBe(true);
    }
  });

  it("gives every configuration key a `roboDresser.` prefix (Req 2.3)", () => {
    const properties = packageJson.contributes.configuration.properties;
    const keys = Object.keys(properties);
    expect(keys.length).toBeGreaterThan(0);
    for (const key of keys) {
      expect(key.startsWith("roboDresser.")).toBe(true);
    }
  });

  it("declares roboDresser.port's schema as minimum 1, maximum 65535 (Req 5.2)", () => {
    const portSchema = packageJson.contributes.configuration.properties["roboDresser.port"];
    expect(portSchema).toBeDefined();
    expect(portSchema.minimum).toBe(1);
    expect(portSchema.maximum).toBe(65535);
  });
});
