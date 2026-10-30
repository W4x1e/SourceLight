import { useState } from "react";
import { t, type Key } from "../../src/i18n";
import { api, type LinkStatus } from "./api";

const DEFAULT_NAME: Record<LinkStatus["platform"], Key> = {
  android: "mobile.pair.defaultAndroid",
  ios: "mobile.pair.defaultIos",
  other: "mobile.pair.defaultOther",
};

/** What the computer calls this device until the person names it: an iPad
 *  is an iPad, though iOS reports both as "ios". */
function defaultName(platform: LinkStatus["platform"]): Key {
  if (platform === "ios" && !/iPhone|iPod/.test(navigator.userAgent)) return "mobile.pair.defaultIpad";
  return DEFAULT_NAME[platform];
}

/**
 * Pairing (docs/MOBILE.md §3): scan the code the desktop shows, or paste its
 * link, then wait for someone at the desktop to allow this phone.
 */
export default function Pair({
  platform,
  onPaired,
  onSkip,
}: {
  platform: LinkStatus["platform"];
  onPaired: () => void;
  onSkip: () => void;
}) {
  const [code, setCode] = useState("");
  const [name, setName] = useState(() => t(defaultName(platform)));
  const [waiting, setWaiting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const pair = async (link: string) => {
    setWaiting(true);
    setError(null);
    try {
      await api.pair(link, name.trim() || t(defaultName(platform)));
      onPaired();
    } catch (e) {
      setError(String(e));
    } finally {
      setWaiting(false);
    }
  };

  const scan = async () => {
    setError(null);
    try {
      const link = await api.scanQr(t("mobile.pair.scanPrompt"));
      if (!link) return;
      setCode(link);
      await pair(link);
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <main className="flex min-h-full flex-col gap-5 bg-base px-5 pt-[calc(var(--safe-top)+2rem)] pb-[calc(var(--safe-bottom)+2rem)] md:px-[max(1.25rem,calc(50%-18rem))]">
      <div>
        <h1 className="font-display text-display text-ink">{t("mobile.pair.title")}</h1>
        <p className="mt-2 text-body leading-relaxed text-muted">{t("mobile.pair.steps")}</p>
      </div>

      {/* The scanner is Android only so far; on iOS it would only say it needs a phone. */}
      {platform === "android" && (
        <button className="btn-accent min-h-12 text-body" disabled={waiting} onClick={() => void scan()}>
          {t("mobile.pair.scan")}
        </button>
      )}

      <label className="block">
        <span className="text-caption text-muted">
          {t(platform === "android" ? "mobile.pair.orPaste" : "mobile.pair.paste")}
        </span>
        <textarea
          className="input mt-1 h-20 w-full resize-none font-mono text-caption"
          value={code}
          placeholder={t("mobile.pair.pastePlaceholder")}
          autoCapitalize="off"
          autoCorrect="off"
          spellCheck={false}
          onChange={(e) => setCode(e.target.value)}
        />
      </label>

      <label className="block">
        <span className="text-caption text-muted">{t("mobile.pair.deviceName")}</span>
        <input className="input mt-1 min-h-12 w-full text-body" value={name} onChange={(e) => setName(e.target.value)} />
      </label>

      <button
        className="btn-solid min-h-12 text-body"
        disabled={waiting || code.trim() === ""}
        onClick={() => void pair(code.trim())}
      >
        {t("mobile.pair.submit")}
      </button>

      {waiting && <p className="text-caption text-muted">{t("mobile.pair.waiting")}</p>}
      {error && <p className="text-caption text-danger">{error}</p>}

      <button className="btn-ghost mt-auto min-h-12 self-center text-caption text-muted" onClick={onSkip}>
        {t("mobile.pair.skip")}
      </button>
    </main>
  );
}
