// Hand-drawn 16px icon set, 1.5px strokes, so the app doesn't look like every other Lucide UI.
import type { SVGProps } from "react";

type P = SVGProps<SVGSVGElement> & { size?: number };

const base = ({ size = 16, ...rest }: P) => ({
  width: size,
  height: size,
  viewBox: "0 0 16 16",
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 1.5,
  strokeLinecap: "round" as const,
  strokeLinejoin: "round" as const,
  "aria-hidden": true,
  ...rest,
});

export const PlusIcon = (p: P) => (
  <svg {...base(p)}><path d="M8 3.5v9M3.5 8h9" /></svg>
);
export const CloseIcon = (p: P) => (
  <svg {...base(p)}><path d="M4.5 4.5l7 7M11.5 4.5l-7 7" /></svg>
);
export const ChevronIcon = (p: P) => (
  <svg {...base(p)}><path d="M6 4l4 4-4 4" /></svg>
);
export const TableIcon = (p: P) => (
  <svg {...base(p)}><rect x="2.5" y="3" width="11" height="10" rx="1.5" /><path d="M2.5 6.5h11M6.5 6.5V13" /></svg>
);
export const ViewIcon = (p: P) => (
  <svg {...base(p)}><rect x="2.5" y="3" width="11" height="10" rx="1.5" strokeDasharray="2 1.6" /><path d="M2.5 6.5h11" /></svg>
);
export const DatabaseIcon = (p: P) => (
  <svg {...base(p)}><ellipse cx="8" cy="4" rx="5" ry="1.8" /><path d="M3 4v8c0 1 2.2 1.8 5 1.8s5-.8 5-1.8V4M3 8c0 1 2.2 1.8 5 1.8S13 9 13 8" /></svg>
);
export const PlayIcon = (p: P) => (
  <svg {...base(p)}><path d="M5 3.5v9l7-4.5z" fill="currentColor" /></svg>
);
export const StopIcon = (p: P) => (
  <svg {...base(p)}><rect x="4" y="4" width="8" height="8" rx="1.2" fill="currentColor" /></svg>
);
export const RefreshIcon = (p: P) => (
  <svg {...base(p)}><path d="M12.5 6.5A4.6 4.6 0 0 0 4 5M3.5 9.5A4.6 4.6 0 0 0 12 11M4 2.5V5h2.5M12 13.5V11H9.5" /></svg>
);
export const MoreIcon = (p: P) => (
  <svg {...base(p)}><circle cx="4" cy="8" r=".9" fill="currentColor" /><circle cx="8" cy="8" r=".9" fill="currentColor" /><circle cx="12" cy="8" r=".9" fill="currentColor" /></svg>
);
export const CheckIcon = (p: P) => (
  <svg {...base(p)}><path d="M3.5 8.5l3 3 6-7" /></svg>
);
export const AlertIcon = (p: P) => (
  <svg {...base(p)}><path d="M8 2.5l6 10.5H2z" /><path d="M8 7v2.5M8 11.3v.2" /></svg>
);
export const LockIcon = (p: P) => (
  <svg {...base(p)}><rect x="3.5" y="7" width="9" height="6.5" rx="1.3" /><path d="M5.5 7V5.2a2.5 2.5 0 0 1 5 0V7" /></svg>
);
export const SearchIcon = (p: P) => (
  <svg {...base(p)}><circle cx="7" cy="7" r="4" /><path d="M10 10l3.5 3.5" /></svg>
);
export const Spinner = ({ size = 14 }: { size?: number }) => (
  <svg width={size} height={size} viewBox="0 0 16 16" aria-label="Yükleniyor" style={{ animation: "spin 0.8s linear infinite" }}>
    <circle cx="8" cy="8" r="6" fill="none" stroke="currentColor" strokeOpacity=".25" strokeWidth="2" />
    <path d="M14 8a6 6 0 0 0-6-6" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
  </svg>
);
export const PanelIcon = (p: P) => (
  <svg {...base(p)}><rect x="2.5" y="3" width="11" height="10" rx="1.5" /><path d="M10 3v10" /></svg>
);
export const SettingsIcon = (p: P) => (
  <svg {...base(p)}><circle cx="8" cy="8" r="2" /><path d="M8 1.8v1.7M8 12.5v1.7M14.2 8h-1.7M3.5 8H1.8M12.4 3.6l-1.2 1.2M4.8 11.2l-1.2 1.2M12.4 12.4l-1.2-1.2M4.8 4.8L3.6 3.6" /></svg>
);
export const CodeIcon = (p: P) => (
  <svg {...base(p)}><path d="M5.5 4.5L2 8l3.5 3.5M10.5 4.5L14 8l-3.5 3.5" /></svg>
);
export const HomeIcon = (p: P) => (
  <svg {...base(p)}><path d="M2.5 7.5L8 3l5.5 4.5V13h-3.5V9.5h-4V13H2.5z" /></svg>
);
