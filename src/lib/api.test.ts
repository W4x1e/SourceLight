import { describe, expect, it } from "vitest";
import { CoreError } from "./api";

describe("CoreError", () => {
  it("reads as its message alone where a failure is put into words", () => {
    // `String(e)` is how callers say what went wrong; it said "Error: …".
    const e = new CoreError("The file is in use by another program.", "file in use");
    expect(String(e)).toBe("The file is in use by another program.");
    expect(`That didn't work: ${e}`).toBe("That didn't work: The file is in use by another program.");
    expect(e.english).toBe("file in use");
    expect(e instanceof Error).toBe(true);
  });
});
