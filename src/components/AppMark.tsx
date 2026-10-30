/**
 * SourceLight's mark: the app icon's 3×3 grid, lit from the top-left
 * (app-icon.svg), in the same greys. One drawing for every place the app
 * shows its own name — the mode rail and the welcome screen — so the two
 * can't drift apart; the welcome screen drew a four-point sparkle, the
 * glyph DESIGN.md keeps out of the interface, and not this.
 */
export default function AppMark({ size = 22 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden="true">
      <rect x="1.5" y="1.5" width="6" height="6" rx="1" fill="#f4f4f2" />
      <rect x="9" y="1.5" width="6" height="6" rx="1" fill="#b9bcc2" />
      <rect x="16.5" y="1.5" width="6" height="6" rx="1" fill="#7d828b" />
      <rect x="1.5" y="9" width="6" height="6" rx="1" fill="#b9bcc2" />
      <rect x="9" y="9" width="6" height="6" rx="1" fill="#7d828b" />
      <rect x="16.5" y="9" width="6" height="6" rx="1" fill="#4a4e56" />
      <rect x="1.5" y="16.5" width="6" height="6" rx="1" fill="#7d828b" />
      <rect x="9" y="16.5" width="6" height="6" rx="1" fill="#4a4e56" />
      <rect x="16.5" y="16.5" width="6" height="6" rx="1" fill="#2a2c31" />
    </svg>
  );
}
