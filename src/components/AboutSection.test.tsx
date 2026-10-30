import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../lib/api", () => ({
  api: {
    checkForUpdate: vi.fn(),
    installUpdate: vi.fn(),
    licenceText: vi.fn(),
    thirdPartyLicences: vi.fn(),
  },
}));

import { api } from "../lib/api";
import { useStore } from "../store";
import AboutSection from "./AboutSection";

const checkForUpdate = vi.mocked(api.checkForUpdate);
const installUpdate = vi.mocked(api.installUpdate);
const licenceText = vi.mocked(api.licenceText);
const thirdPartyLicences = vi.mocked(api.thirdPartyLicences);

/**
 * Every state the updates box can be in. The app promises to work fully
 * offline, so the ones that matter most are the failures: a check that cannot
 * reach the server must read as "you may be offline", never as something broken.
 */
describe("AboutSection", () => {
  beforeEach(() => {
    useStore.setState({ update: null });
    vi.spyOn(console, "warn").mockImplementation(() => {});
  });
  afterEach(() => {
    vi.clearAllMocks();
    vi.restoreAllMocks();
  });

  it("says plainly when this build has no update source", () => {
    useStore.setState({ update: { configured: false, currentVersion: "0.1.0", available: null } });
    render(<AboutSection />);
    expect(screen.getByText(/can't update itself/)).toBeTruthy();
    expect(screen.queryByText("Check for updates")).toBeNull();
  });

  it("reports the newest version after a check that finds nothing", async () => {
    checkForUpdate.mockResolvedValue({ configured: true, currentVersion: "0.1.0", available: null });
    render(<AboutSection />);
    fireEvent.click(screen.getByText("Check for updates"));
    expect(await screen.findByText("You have the newest version (0.1.0).")).toBeTruthy();
  });

  it("treats an unreachable server as being offline, not as a fault", async () => {
    checkForUpdate.mockRejectedValue("error sending request");
    render(<AboutSection />);
    fireEvent.click(screen.getByText("Check for updates"));
    expect(await screen.findByText(/You may be offline/)).toBeTruthy();
    expect(screen.queryByText(/error sending request/)).toBeNull();
  });

  it("offers an available update with its notes, and installs it", async () => {
    useStore.setState({
      update: {
        configured: true,
        currentVersion: "0.1.0",
        available: { version: "0.2.0", notes: "### Fixed\n- the crop box", date: null },
      },
    });
    installUpdate.mockReturnValue(new Promise(() => {})); // the app exits mid-install
    render(<AboutSection />);
    expect(screen.getByText(/Version 0.2.0 is available/)).toBeTruthy();
    expect(screen.getByText(/the crop box/)).toBeTruthy();
    fireEvent.click(screen.getByText("Install and restart"));
    expect(installUpdate).toHaveBeenCalledOnce();
    expect(await screen.findByText("Installing…")).toBeTruthy();
  });

  it("asks before an install would cut off an export", async () => {
    useStore.setState({
      update: {
        configured: true,
        currentVersion: "0.1.0",
        available: { version: "0.2.0", notes: null, date: null },
      },
      exportJob: {
        request: { ids: [1, 2], files: ["a.jpg", "b.jpg"], directory: "", options: {} as never, rename: null, subfolder: null },
        done: 1,
        message: "a.jpg",
        running: true,
        summary: null,
        results: {},
        directory: null,
      },
    });
    render(<AboutSection />);
    fireEvent.click(screen.getByText("Install and restart"));
    await Promise.resolve();
    expect(installUpdate).not.toHaveBeenCalled();
    useStore.setState({ exportJob: null });
  });

  it("says so when an install fails, and lets it be retried", async () => {
    useStore.setState({
      update: {
        configured: true,
        currentVersion: "0.1.0",
        available: { version: "0.2.0", notes: null, date: null },
      },
    });
    // `api` wraps command errors in `Error` — the message must read as a
    // sentence, not as "Error: …".
    installUpdate.mockRejectedValue(new Error("signature did not verify"));
    render(<AboutSection />);
    fireEvent.click(screen.getByText("Install and restart"));
    expect(await screen.findByText("The update didn't install: signature did not verify")).toBeTruthy();
    expect((screen.getByText("Install and restart") as HTMLButtonElement).disabled).toBe(false);
  });

  it("opens SourceLight's licence and the third-party licences", async () => {
    useStore.setState({ update: { configured: false, currentVersion: "0.1.0", available: null } });
    licenceText.mockResolvedValue("Mozilla Public License Version 2.0");
    thirdPartyLicences.mockResolvedValue("MIT License — some crate");
    render(<AboutSection />);

    expect(screen.getByText(/open source, under the Mozilla Public License 2\.0/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "SourceLight licence" }));
    expect(await screen.findByText("Mozilla Public License Version 2.0")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(screen.queryByText("Mozilla Public License Version 2.0")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Third-party licences" }));
    expect(await screen.findByText("MIT License — some crate")).toBeTruthy();
  });
});
