// One stroke icon set on an 18px grid, 1.6px strokes, round caps. The Rust port bundles these
// same paths as SVG assets so the app and the prototype stay identical.

const STROKE: Record<string, string> = {
  search: "M8 12.5a4.5 4.5 0 1 0 0-9 4.5 4.5 0 0 0 0 9ZM11.5 11.5 15 15",
  plus: "M9 4v10M4 9h10",
  chevron: "M5 7l4 4 4-4",
  chevronRight: "M7 5l4 4-4 4",
  close: "M5 5l8 8M13 5l-8 8",
  sidebar: "M4.5 3h9a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2h-9a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2ZM7 3v12",
  folder: "M2.5 5.5a2 2 0 0 1 2-2h3l1.5 1.5h4.5a2 2 0 0 1 2 2v5.5a2 2 0 0 1-2 2h-9a2 2 0 0 1-2-2Z",
  file: "M5 2.5h5l3.5 3.5v8a1.5 1.5 0 0 1-1.5 1.5H5A1.5 1.5 0 0 1 3.5 14V4A1.5 1.5 0 0 1 5 2.5ZM10 2.5V6h3.5",
  phone: "M7.5 2.5h3a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2h-3a2 2 0 0 1-2-2v-9a2 2 0 0 1 2-2ZM8 13h2",
  check: "M4 9.5l3 3 7-7",
  bookmark: "M5.5 3h7v12l-3.5-2.5L5.5 15Z",
  fork: "M5 6v1.5a2 2 0 0 0 2 2h4a2 2 0 0 0 2-2V6M9 9.5V12M5 4.5h.01M13 4.5h.01M9 13.5h.01",
  compact: "M4 7h10M4 11h10M7 3.5l2 2 2-2M7 14.5l2-2 2 2",
  play: "M6 4.5v9l7-4.5Z",
  pr: "M5 3.5v11M13 14.5V8a2 2 0 0 0-2-2H8M10 4 8 6l2 2",
  push: "M9 14V4M5 8l4-4 4 4",
  pencil: "M11.5 3.5l3 3L7 14H4v-3Z",
  history: "M3.5 9a5.5 5.5 0 1 0 1.6-3.9M3.5 3.5v2.6h2.6M9 6.5V9l2 1.5",
  gear: "M9 11.2a2.2 2.2 0 1 0 0-4.4 2.2 2.2 0 0 0 0 4.4ZM9 2.5v1.6M9 13.9v1.6M15.5 9h-1.6M4.1 9H2.5M13.6 4.4l-1.1 1.1M5.5 12.5l-1.1 1.1M13.6 13.6l-1.1-1.1M5.5 5.5 4.4 4.4",
  terminal: "M3.5 3.5h11a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1v-9a1 1 0 0 1 1-1ZM5.5 7l2 2-2 2M9.5 11h3",
  pin: "M7 2.5h4l-.5 4 2.5 2.5H5l2.5-2.5ZM9 9v6.5",
  splitRight: "M3.5 3.5h11a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1v-9a1 1 0 0 1 1-1ZM9 3.5v11",
  splitDown: "M3.5 3.5h11a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1v-9a1 1 0 0 1 1-1ZM2.5 9h13",
  transfer: "M4 6.5h9.5L11 4M14 11.5H4.5L7 14",
  copy: "M6.5 6.5h7a1 1 0 0 1 1 1v6a1 1 0 0 1-1 1h-6a1 1 0 0 1-1-1ZM11.5 6.5v-2a1 1 0 0 0-1-1h-6a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2",
  trash: "M3.5 5h11M7 5V3.5h4V5M5 5l.7 9.1a1 1 0 0 0 1 .9h4.6a1 1 0 0 0 1-.9L13 5",
  zap: "M10 2.5 4.5 10H9l-1 5.5L13.5 8H9Z",
  keyboard: "M3 5h12a1 1 0 0 1 1 1v6a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1ZM5 8h.01M7.5 8h.01M10 8h.01M12.5 8h.01M6 10.5h6",
  palette: "M9 15.5a6.5 6.5 0 1 1 6.5-6.5c0 1.7-1.3 2.5-2.6 2.5h-1.5a1.4 1.4 0 0 0-1 2.4 1 1 0 0 1-.7 1.6ZM5.8 8.2h.01M8 5.6h.01M11 5.8h.01",
  bell: "M5 12.5V8a4 4 0 0 1 8 0v4.5l1.2 1.5H3.8ZM7.5 15.5a1.6 1.6 0 0 0 3 0",
  lock: "M5 8h8a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V9a1 1 0 0 1 1-1ZM6.5 8V6a2.5 2.5 0 0 1 5 0v2",
  info: "M9 15.5a6.5 6.5 0 1 0 0-13 6.5 6.5 0 0 0 0 13ZM9 8.5v3.5M9 6h.01",
  puzzle: "M7.5 3.5a1.5 1.5 0 0 1 3 0v1h3v3h-1a1.5 1.5 0 0 0 0 3h1v3h-3v-1a1.5 1.5 0 0 0-3 0v1h-3v-3h1a1.5 1.5 0 0 0 0-3h-1v-3h3Z",
  code: "M6.5 5 2.5 9l4 4M11.5 5l4 4-4 4",
  branch: "M5 3.5v11M5 10.5a4 4 0 0 0 4-4h1M12 4.5a2 2 0 1 0 0 4 2 2 0 0 0 0-4Z",
  sparkle: "M9 2.5 10.4 7.6 15.5 9l-5.1 1.4L9 15.5l-1.4-5.1L2.5 9l5.1-1.4Z",
  gauge: "M3.5 12a5.5 5.5 0 1 1 11 0M9 12l2.5-3",
  eye: "M1.8 9S4.5 4 9 4s7.2 5 7.2 5-2.7 5-7.2 5-7.2-5-7.2-5ZM9 11a2 2 0 1 0 0-4 2 2 0 0 0 0 4Z",
  grip: "M6.5 4.5h.01M6.5 9h.01M6.5 13.5h.01M11.5 4.5h.01M11.5 9h.01M11.5 13.5h.01",
  external: "M10.5 3.5h4v4M14.5 3.5 8.5 9.5M12.5 10.5v3a1 1 0 0 1-1 1h-7a1 1 0 0 1-1-1v-7a1 1 0 0 1 1-1h3",
  warning: "M9 3 16 15H2ZM9 7.5v3.5M9 13h.01",
  sun: "M9 12a3 3 0 1 0 0-6 3 3 0 0 0 0 6ZM9 1.8v1.6M9 14.6v1.6M16.2 9h-1.6M3.4 9H1.8M14.1 3.9 13 5M5 13l-1.1 1.1M14.1 14.1 13 13M5 5 3.9 3.9",
  moon: "M14.5 11A6 6 0 0 1 7 3.5a6 6 0 1 0 7.5 7.5Z",
  layers: "M9 2.5 15.5 6 9 9.5 2.5 6ZM2.5 9.5 9 13l6.5-3.5M2.5 12.5 9 16l6.5-3.5",
  cloud: "M5 14.5h8.5a3 3 0 0 0 .4-6 4.5 4.5 0 0 0-8.7-.9A3.4 3.4 0 0 0 5 14.5Z",
  arrowLeft: "M14 9H4M8 5 4 9l4 4",
  arrowDown: "M9 4v10M5 10l4 4 4-4",
  refresh: "M14.5 9a5.5 5.5 0 1 1-1.6-3.9M14.5 3.5v2.6h-2.6",
  user: "M9 9a3 3 0 1 0 0-6 3 3 0 0 0 0 6ZM3.5 15.5a5.5 5.5 0 0 1 11 0",
  filter: "M3 4h12l-4.5 5.5V14l-3 1.5v-6Z",
  minus: "M4 9h10",
  dots: "",
  claude: "M9 3v12M3 9h12M4.8 4.8l8.4 8.4M13.2 4.8l-8.4 8.4",
  codex: "M9 2.8l5.4 3.1v6.2L9 15.2l-5.4-3.1V5.9ZM9 11a2 2 0 1 0 0-4 2 2 0 0 0 0 4Z",
  command: "M6 6V4.5A1.5 1.5 0 1 0 4.5 6H6Zm0 0h6M6 6v6m6-6V4.5A1.5 1.5 0 1 1 13.5 6H12Zm0 0v6m0 0v1.5a1.5 1.5 0 1 0 1.5-1.5H12Zm0 0H6m0 0v1.5A1.5 1.5 0 1 1 4.5 12H6Z",
  send: "M15.5 2.5 2.5 8l5 2 2 5Z M7.5 10l8-7.5",
  image: "M3.5 3.5h11a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1v-9a1 1 0 0 1 1-1ZM2.5 12l3.5-3.5 3 3 2-2 4.5 4M11.5 7h.01",
};

export type IconName = keyof typeof STROKE | "needs" | "restoring";

export function Icon({
  name,
  size = 16,
  className,
  title,
  strokeWidth = 1.6,
}: {
  name: IconName;
  size?: number;
  className?: string;
  title?: string;
  strokeWidth?: number;
}) {
  const labelled = title !== undefined;
  const a11y = labelled ? { role: "img", "aria-label": title } : { "aria-hidden": true as const };

  if (name === "needs") {
    // Filled circle with a knocked-out "!" so "needs you" differs in shape, not only color.
    return (
      <svg width={size} height={size} viewBox="0 0 18 18" className={className} {...a11y}>
        <circle cx="9" cy="9" r="7" fill="currentColor" />
        <path d="M9 5.2v4.6" stroke="var(--knockout, var(--surface-chrome))" strokeWidth="1.8" strokeLinecap="round" />
        <circle cx="9" cy="12.6" r="1.1" fill="var(--knockout, var(--surface-chrome))" />
      </svg>
    );
  }
  if (name === "restoring") {
    // A dashed ring: static, so the native port needs no animation system to draw it.
    return (
      <svg width={size} height={size} viewBox="0 0 18 18" className={className} fill="none" stroke="currentColor" strokeWidth={1.6} {...a11y}>
        <circle cx="9" cy="9" r="6" strokeDasharray="2.6 2.6" strokeLinecap="round" />
      </svg>
    );
  }
  if (name === "dots") {
    return (
      <svg width={size} height={size} viewBox="0 0 18 18" className={className} fill="currentColor" {...a11y}>
        <circle cx="4.5" cy="9" r="1.2" />
        <circle cx="9" cy="9" r="1.2" />
        <circle cx="13.5" cy="9" r="1.2" />
      </svg>
    );
  }
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 18 18"
      className={className}
      fill="none"
      stroke="currentColor"
      strokeWidth={name === "claude" ? 1.9 : strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      {...a11y}
    >
      <path d={STROKE[name]} />
    </svg>
  );
}
