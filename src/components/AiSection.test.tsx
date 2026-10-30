import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("../lib/api", () => ({
  api: {
    captionStatus: vi.fn(async () => 0),
    searchIndexStatus: vi.fn(async () => 12),
    facesStatus: vi.fn(async () => ({ toDetect: 5, toRecognise: 0, recogniserInstalled: true })),
    aiAvailableModels: vi.fn(async () => [
      { id: "tagger", name: "Search", purpose: "", license: "Apache-2.0", approxMb: 785, installed: true },
      { id: "denoise", name: "Denoise", purpose: "", license: "MIT", approxMb: 120, installed: true },
    ]),
    localVlmStorage: vi.fn(async () => ({ runtimeInstalled: false, runtimeMb: 0, models: [] })),
    vlmCatalogue: vi.fn(async () => []),
    editServerStatus: vi.fn(async () => ({
      enabled: true,
      running: true,
      url: "http://127.0.0.1:47811/mcp",
      command: "C:\\Program Files\\SourceLight\\SourceLight.exe",
      token: "0123abcd",
      error: null,
    })),
  },
}));

import { useStore } from "../store";
import AiSection from "./AiSection";

const ai = { provider: "builtin", configured: true, ready: true, model: "", detail: "", models: [] } as never;

/** Settings › AI holds back only what conflicts, and says why. */
describe("AiSection while something runs", () => {
  beforeEach(() => useStore.setState({ ai, tagging: null, jobs: {}, refreshAi: async () => {} }));

  it("leaves every action free while only a download or a light pass runs", async () => {
    useStore.setState({
      jobs: {
        "model:subject": { label: "Downloading model", current: 1, total: 9, unit: "bytes" },
        colours: { label: "Colours", current: 1, total: 9, unit: "count" },
      },
    });
    render(<AiSection />);
    const index = await screen.findByRole("button", { name: /Index 12/ });
    expect((index as HTMLButtonElement).disabled).toBe(false);
    expect(screen.queryByText(/Waiting for/)).toBeNull();
    for (const b of screen.getAllByRole("button", { name: "Remove" })) {
      expect((b as HTMLButtonElement).disabled).toBe(false);
    }
  });

  it("holds the other rows while a heavy pass runs, saying which", async () => {
    useStore.setState({ jobs: { faces: { label: "Finding faces", current: 1, total: 5, unit: "count" } } });
    render(<AiSection />);
    const index = await screen.findByRole("button", { name: /Index 12/ });
    expect((index as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getAllByText(/Waiting for finding faces to finish/).length).toBeGreaterThan(0);
  });

  it("keeps a model from being removed only while something uses it", async () => {
    useStore.setState({ jobs: { index: { label: "Indexing for search", current: 1, total: 5, unit: "count" } } });
    render(<AiSection />);
    await waitFor(() => expect(screen.getAllByRole("button", { name: "Remove" })).toHaveLength(2));
    const [search, denoise] = screen.getAllByRole("button", { name: "Remove" }) as HTMLButtonElement[];
    expect(search!.disabled).toBe(true);
    expect(search!.title).toMatch(/In use/);
    expect(denoise!.disabled).toBe(false);
  });
});

describe("AiSection's AI apps row", () => {
  beforeEach(() => useStore.setState({ ai, tagging: null, jobs: {}, refreshAi: async () => {} }));

  it("shows the address to give an AI app while the server runs", async () => {
    render(<AiSection />);
    expect(await screen.findByText("http://127.0.0.1:47811/mcp")).toBeTruthy();
    expect(screen.getAllByRole("button", { name: "Copy address" }).length).toBe(1);
    expect(screen.getByText("Authorization: Bearer 0123abcd")).toBeTruthy();
  });

  it("says how to connect each app, with this program for the ones that start a command", async () => {
    render(<AiSection />);
    (await screen.findByRole("button", { name: "How to connect an AI app" })).click();
    const commands = await screen.findAllByText(/--mcp/, { selector: "pre" });
    expect(commands.map((c) => c.textContent)).toContain(
      'codex mcp add sourcelight -- "C:\\Program Files\\SourceLight\\SourceLight.exe" --mcp',
    );
    const desktop = JSON.parse(commands.find((c) => c.textContent!.startsWith("{"))!.textContent!);
    expect(desktop.mcpServers.sourcelight).toEqual({
      command: "C:\\Program Files\\SourceLight\\SourceLight.exe",
      args: ["--mcp"],
    });
    expect(
      screen.getByText(
        'claude mcp add --transport http sourcelight http://127.0.0.1:47811/mcp --header "Authorization: Bearer 0123abcd"',
      ),
    ).toBeTruthy();
    const http = screen.getAllByText(/"headers"/, { selector: "pre" });
    expect(http.map((c) => JSON.parse(c.textContent!).mcpServers.sourcelight)).toEqual([
      { httpUrl: "http://127.0.0.1:47811/mcp", headers: { Authorization: "Bearer 0123abcd" } },
      { url: "http://127.0.0.1:47811/mcp", headers: { Authorization: "Bearer 0123abcd" } },
    ]);
    expect(screen.getAllByRole("button", { name: "Copy address" }).length).toBe(1);
    expect(screen.getAllByRole("button", { name: "Copy" }).length).toBe(6);
  });
});
