import { useCallback, useEffect, useState } from "react";
import { api, type LinkStatus } from "./api";
import Home from "./Home";
import Pair from "./Pair";

/** Pairing until a computer is paired; then the two libraries. */
export default function App() {
  const [status, setStatus] = useState<LinkStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  // The phone's own photos work without a computer.
  const [skipped, setSkipped] = useState(false);

  const refresh = useCallback(() => {
    api
      .linkStatus()
      .then(setStatus)
      .catch((e) => setError(String(e)));
  }, []);
  useEffect(refresh, [refresh]);

  if (error) return <p className="p-6 text-caption text-danger">{error}</p>;
  if (!status) return null;
  if (status.desktop) return <Home key="paired" desktop={status.desktop} platform={status.platform} onPair={refresh} />;
  if (skipped) return <Home key="unpaired" desktop={null} platform={status.platform} onPair={() => setSkipped(false)} />;
  return <Pair platform={status.platform} onPaired={refresh} onSkip={() => setSkipped(true)} />;
}
