import { Icon, type IconName } from "../../lib/icons";
import { t, type Key } from "../../i18n";

/** One tool in the rail's tool switch — icon over its name, since the icons
 *  alone were hard to tell apart. `marker` flags a tool holding edits: a dot,
 *  or a count. */
export default function ToolButton({
  icon,
  label,
  active,
  onClick,
  title,
  marker = false,
}: {
  icon: IconName;
  label: Key;
  active: boolean;
  onClick: () => void;
  title: string;
  marker?: boolean | number;
}) {
  return (
    <button
      onClick={onClick}
      title={title}
      aria-pressed={active}
      className={`tool-btn relative ${active ? "tool-on" : ""}`}
    >
      <Icon name={icon} size={16} />
      {t(label)}
      {/* Beside its own icon, not in the button's far corner: there a crop's
          dot sat against the Mask button and read as the mask's. */}
      {marker !== false && (
        <span className="absolute left-1/2 top-1.5 ml-2.5 text-label font-medium leading-none tabular-nums text-muted">
          {marker === true ? <span className="block h-1.5 w-1.5 rounded-full bg-accent" /> : marker}
        </span>
      )}
    </button>
  );
}
