import { useEffect, useState } from "react";
import { t } from "../../src/i18n";
import { PanelHeader } from "./ui";

/**
 * The phone app's third-party notices (mobile/THIRD-PARTY-LICENSES.txt), as
 * their licences ask, from the app's menu. The file is large, so it loads
 * only when this opens.
 */
export default function Licences({ onClose }: { onClose: () => void }) {
  const [text, setText] = useState<string | null>(null);
  useEffect(() => {
    void import("../THIRD-PARTY-LICENSES.txt?raw").then((m) => setText(m.default));
  }, []);
  return (
    <div className="flex h-full flex-col bg-base">
      <PanelHeader title={t("mobile.menu.licences")} onClose={onClose} />
      <pre className="min-h-0 flex-1 overflow-auto px-4 pt-3 pb-[calc(var(--safe-bottom)+1rem)] font-mono text-label leading-relaxed break-words whitespace-pre-wrap text-muted">
        {text ?? t("common.loading")}
      </pre>
    </div>
  );
}
