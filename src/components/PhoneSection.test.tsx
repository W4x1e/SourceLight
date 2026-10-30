import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("../lib/api", () => ({
  api: {
    phoneLinkStatus: vi.fn(),
    setPhoneLink: vi.fn(),
    startPhonePairing: vi.fn(),
    cancelPhonePairing: vi.fn(),
    answerPhonePairing: vi.fn(),
    removePhone: vi.fn(),
    phoneImportFolder: vi.fn(() => Promise.resolve({ path: "D:\Photos\From phone", chosen: false })),
    setPhoneImportFolder: vi.fn(() => Promise.resolve()),
  },
}));
vi.mock("../lib/dialogs", () => ({ confirm: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

import { open as openDirectoryDialog } from "@tauri-apps/plugin-dialog";
import { api } from "../lib/api";
import { confirm } from "../lib/dialogs";
import type { PhoneLinkStatus } from "../bindings";
import PhoneSection from "./PhoneSection";

const status = (over: Partial<PhoneLinkStatus> = {}): PhoneLinkStatus => ({
  available: true,
  enabled: true,
  awayFromHome: true,
  running: true,
  error: null,
  phones: [],
  ...over,
});

const pairing = {
  uri: "sourcelight://pair#abc",
  qrSvg: "<svg xmlns='http://www.w3.org/2000/svg'></svg>",
  expiresInSecs: 300,
  reachableAway: true,
};

/** Settings → Phone (docs/MOBILE.md). */
describe("PhoneSection", () => {
  afterEach(() => vi.clearAllMocks());

  it("shows the switches and says when no phone is paired", async () => {
    vi.mocked(api.phoneLinkStatus).mockResolvedValue(status({ enabled: false }));
    render(<PhoneSection />);
    expect(await screen.findByText("Phone connections")).toBeTruthy();
    expect(screen.getByText("Away from home")).toBeTruthy();
    expect(screen.getByText("No phones paired yet.")).toBeTruthy();
    expect(screen.getByText("Pair a phone")).toBeTruthy();
  });

  // Add to library (docs/MOBILE.md section 4): where phone photos are saved.
  it("says where photos from a phone go, and lets that change", async () => {
    vi.mocked(api.phoneLinkStatus).mockResolvedValue(status());
    vi.mocked(openDirectoryDialog).mockResolvedValue("E:\Phone photos");
    render(<PhoneSection />);
    expect(await screen.findByText("D:\Photos\From phone")).toBeTruthy();
    expect(screen.queryByText("Use the default")).toBeNull();
    fireEvent.click(screen.getByText("Change…"));
    await waitFor(() => expect(api.setPhoneImportFolder).toHaveBeenCalledWith("E:\Phone photos"));
  });

  it("shows a pairing code, and cancelling retires it", async () => {
    vi.mocked(api.phoneLinkStatus).mockResolvedValue(status());
    vi.mocked(api.startPhonePairing).mockResolvedValue(pairing);
    vi.mocked(api.cancelPhonePairing).mockResolvedValue(null);
    render(<PhoneSection />);
    fireEvent.click(await screen.findByText("Pair a phone"));
    expect(await screen.findByAltText("Pairing code")).toBeTruthy();
    expect(screen.getByText("It works once and expires in 5 minutes.")).toBeTruthy();
    expect(screen.queryByText(/only works on this network/)).toBeNull();

    fireEvent.click(screen.getByText("Cancel"));
    expect(api.cancelPhonePairing).toHaveBeenCalledOnce();
    expect(screen.queryByAltText("Pairing code")).toBeNull();
  });

  it("says when a code only works on this network", async () => {
    vi.mocked(api.phoneLinkStatus).mockResolvedValue(status({ awayFromHome: false }));
    vi.mocked(api.startPhonePairing).mockResolvedValue({ ...pairing, reachableAway: false });
    render(<PhoneSection />);
    fireEvent.click(await screen.findByText("Pair a phone"));
    expect(await screen.findByText(/only works on this network/)).toBeTruthy();
  });

  it("lists paired phones and removes one after asking", async () => {
    const phones = [
      { id: "a", name: "James's iPhone", platform: "ios", pairedAt: "2026-10-02 10:00:00", lastSeen: "2026-10-02 11:00:00" },
      { id: "b", name: "Pixel", platform: "android", pairedAt: "2026-10-02 10:05:00", lastSeen: null },
    ];
    vi.mocked(api.phoneLinkStatus).mockResolvedValue(status({ phones }));
    vi.mocked(api.removePhone).mockResolvedValue(status({ phones: [phones[1]!] }));
    vi.mocked(confirm).mockResolvedValue(true);
    render(<PhoneSection />);

    const iphone = (await screen.findByText("James's iPhone")).closest("li")!;
    expect(within(iphone).getByText(/iPhone or iPad/)).toBeTruthy();
    expect(within(iphone).getByText(/Last connected/)).toBeTruthy();
    const pixel = screen.getByText("Pixel").closest("li")!;
    expect(within(pixel).getByText(/Not connected yet/)).toBeTruthy();

    fireEvent.click(within(iphone).getByText("Remove"));
    expect(confirm).toHaveBeenCalledWith(expect.objectContaining({ title: "Remove “James's iPhone”?", danger: true }));
    await screen.findByText("Pixel");
    expect(api.removePhone).toHaveBeenCalledWith("a");
    expect(await screen.findByText("Pixel")).toBeTruthy();
    expect(screen.queryByText("James's iPhone")).toBeNull();
  });

  it("keeps a phone when removal is cancelled", async () => {
    vi.mocked(api.phoneLinkStatus).mockResolvedValue(
      status({ phones: [{ id: "a", name: "Phone", platform: "other", pairedAt: "x", lastSeen: null }] }),
    );
    vi.mocked(confirm).mockResolvedValue(false);
    render(<PhoneSection />);
    fireEvent.click(await screen.findByText("Remove"));
    await waitFor(() => expect(confirm).toHaveBeenCalledOnce());
    expect(api.removePhone).not.toHaveBeenCalled();
    expect(screen.getByText("Phone")).toBeTruthy();
  });

  it("turns connections off and keeps the away-from-home choice", async () => {
    vi.mocked(api.phoneLinkStatus).mockResolvedValue(status({ awayFromHome: false }));
    vi.mocked(api.setPhoneLink).mockResolvedValue(status({ enabled: false, awayFromHome: false, running: false }));
    render(<PhoneSection />);
    const group = await screen.findByRole("radiogroup", { name: "Phone connections" });
    fireEvent.click(within(group).getByText("Off"));
    expect(api.setPhoneLink).toHaveBeenCalledWith(false, false);
    await waitFor(() => expect(within(group).getByText("Off").getAttribute("aria-checked")).toBe("true"));
  });

  it("says why the link did not start", async () => {
    vi.mocked(api.phoneLinkStatus).mockResolvedValue(
      status({ running: false, error: "address in use" }),
    );
    render(<PhoneSection />);
    expect(await screen.findByText("Phone connections couldn't start: address in use")).toBeTruthy();
  });
});
