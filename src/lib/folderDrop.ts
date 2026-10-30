import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { api } from "./api";
import { useStore } from "../store";

/**
 * Folders dropped on the window join the library, as a folder dragged onto
 * a Mac app's window is expected to — from Finder, or a card's DCIM folder
 * from the desktop. Each goes through the same path as the folder picker, so
 * it is scanned straight away, and a dropped photo gets the backend's "isn't a
 * folder" rather than being added as one.
 *
 * So do folders dropped on the Dock icon. Those arrive from the backend
 * (`folders-opened`), which holds them until asked: a drop that launched the
 * app came in before anything here was listening, so they are also taken once
 * at start.
 */
export function installFolderDrop(): void {
  if (!("__TAURI_INTERNALS__" in window)) return;
  const add = async (paths: string[]) => {
    for (const path of paths) await useStore.getState().addLibraryFolder(path);
  };
  void getCurrentWebview().onDragDropEvent(async (e) => {
    if (e.payload.type !== "drop") return;
    await add(e.payload.paths);
  });
  const takeOpened = async () => add(await api.takeOpenedFolders().catch(() => []));
  void listen("folders-opened", () => void takeOpened());
  void takeOpened();
}
