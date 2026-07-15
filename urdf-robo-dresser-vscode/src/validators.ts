/**
 * Pure validators for Robo Dresser configuration settings.
 */

/** Req 5.9: gate a start attempt on the configured port being 1-65535. */
export function isValidPort(port: number): boolean {
  return Number.isInteger(port) && port >= 1 && port <= 65535;
}

/**
 * Req 5.8: check catalogue -> binaryPath -> webRoot in order, report only the first invalid one.
 */
export function firstInvalidPathSetting(
  settings: { catalogue: string; binaryPath: string; webRoot: string },
  exists: (p: string) => boolean
): "catalogue" | "binaryPath" | "webRoot" | undefined {
  const order: Array<"catalogue" | "binaryPath" | "webRoot"> = [
    "catalogue",
    "binaryPath",
    "webRoot",
  ];

  for (const setting of order) {
    if (!exists(settings[setting])) {
      return setting;
    }
  }

  return undefined;
}
