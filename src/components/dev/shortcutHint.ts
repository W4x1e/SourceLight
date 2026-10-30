import { comboToLabel } from "../../lib/keymap";
import { t } from "../../i18n";

/** A control's name with its keyboard shortcut, when it has one. */
export const hint = (label: string, combo?: Parameters<typeof comboToLabel>[0]) =>
  combo ? t("develop.shortcutHint", { label, key: comboToLabel(combo) }) : label;
