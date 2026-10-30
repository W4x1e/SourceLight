import { describe, expect, it } from "vitest";
import { matchScore, rankCommands, type Command } from "./palette";

const cmd = (label: string, aliases?: string[]): Command => ({
  id: label,
  label,
  group: "App",
  aliases,
  run: () => {},
});
const labels = (list: Command[]) => list.map((c) => c.label);

describe("the command palette's matching", () => {
  it("finds letters in order, not together, and nothing out of order", () => {
    expect(matchScore("exsel", "Export selected…")).not.toBeNull();
    expect(matchScore("sel ex", "Export selected…")).toBeNull();
    expect(matchScore("zzz", "Export selected…")).toBeNull();
    expect(matchScore("", "anything")).toBe(0);
  });

  it("puts word starts and unbroken runs first", () => {
    const list = [
      cmd("Accept the suggested pick or reject in Cull"),
      cmd("Go to Cull"),
      cmd("Clear rating"),
    ];
    expect(labels(rankCommands(list, "cull"))[0]).toBe("Go to Cull");
    expect(labels(rankCommands(list, "clear"))[0]).toBe("Clear rating");
  });

  it("answers to aliases, and lists everything, in order, for nothing typed", () => {
    const list = [cmd("Show photos of Ada", ["Ada Lovelace", "person"]), cmd("Go to Browse")];
    expect(labels(rankCommands(list, "lovelace"))).toEqual(["Show photos of Ada"]);
    expect(labels(rankCommands(list, ""))).toEqual(["Show photos of Ada", "Go to Browse"]);
    expect(rankCommands(list, "qqq")).toEqual([]);
  });
});
