import { prompt } from "./dialogs";
import { t } from "../i18n";

/**
 * Develop's "Save as copy": asks for a name, then `save`s — or does nothing
 * at all when the question is cancelled. Cancel and Esc made the copy
 * anyway, reset the original to how it was when opened and moved Develop onto
 * the copy, because `prompt` answered null for both a cancel and an empty
 * name. An empty name is still fine: the badge and the place beside the
 * original say what it is.
 */
export async function askToSaveAsCopy(save: (name: string | null) => Promise<void>): Promise<void> {
  const name = await prompt({
    title: t("develop.copyAsk.title"),
    message: t("develop.copyAsk.message"),
    placeholder: t("develop.copyAsk.placeholder"),
    confirmLabel: t("develop.copyAsk.confirm"),
    allowEmpty: true,
  });
  if (name === null) return;
  await save(name || null);
}
