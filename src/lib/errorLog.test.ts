import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("./api", () => ({ api: { logUiError: vi.fn() } }));

import { api } from "./api";
import { reportUiError } from "./errorLog";

const log = vi.mocked(api.logUiError);

/** Interface errors written to the log folder. */
describe("reportUiError", () => {
  afterEach(() => vi.clearAllMocks());

  it("sends the error's name, message and stack, with the component stack after", () => {
    log.mockResolvedValue(null);
    const err = new TypeError("x is undefined");
    reportUiError("Develop", err, "    at Develop\n    at App");

    const [surface, message, stack] = log.mock.calls[0]!;
    expect(surface).toBe("Develop");
    expect(message).toBe("TypeError: x is undefined");
    expect(stack).toContain("x is undefined"); // the JS stack
    expect(stack).toContain("component stack:\n    at Develop");
  });

  it("logs a thrown non-Error as text", () => {
    log.mockResolvedValue(null);
    reportUiError("window", "a plain string was thrown");
    expect(log.mock.calls[0]![1]).toBe("a plain string was thrown");
  });

  it("never becomes a second error when logging itself fails", async () => {
    log.mockRejectedValue(new Error("backend unavailable"));
    expect(() => reportUiError("window", new Error("first"))).not.toThrow();
    // Let the rejected promise settle: an unhandled rejection here would fail the run.
    await new Promise((r) => setTimeout(r, 0));

    log.mockImplementation(() => {
      throw new Error("no bridge at all");
    });
    expect(() => reportUiError("window", new Error("first"))).not.toThrow();
  });
});
