// Line icons, 24x24, drawn with the current text colour.

type P = { className?: string };
const svg = (path: React.ReactNode) => (props: P) => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.8} strokeLinecap="round" strokeLinejoin="round" {...props}>
    {path}
  </svg>
);

export const DownloadIcon = svg(
  <>
    <path d="M12 4v11" />
    <path d="m7 10 5 5 5-5" />
    <path d="M5 20h14" />
  </>,
);
export const EditIcon = svg(
  <>
    <rect x="3" y="5" width="18" height="14" rx="2" />
    <path d="M3 15h18M8 15v4M16 15v4" />
  </>,
);
export const TasksIcon = svg(
  <>
    <path d="M9 6h11M9 12h11M9 18h11" />
    <path d="m3.5 6 1.2 1.2L7 5M3.5 12l1.2 1.2L7 11M3.5 18l1.2 1.2L7 17" />
  </>,
);
export const SetupIcon = svg(
  <>
    <circle cx="12" cy="12" r="3" />
    <path d="M12 2v3M12 19v3M4.9 4.9l2.1 2.1M17 17l2.1 2.1M2 12h3M19 12h3M4.9 19.1 7 17M17 7l2.1-2.1" />
  </>,
);
export const PlayIcon = svg(<path d="M8 5.5v13l10.5-6.5z" fill="currentColor" stroke="none" />);
export const PauseIcon = svg(
  <>
    <rect x="6.5" y="5" width="3.5" height="14" rx="1" fill="currentColor" stroke="none" />
    <rect x="14" y="5" width="3.5" height="14" rx="1" fill="currentColor" stroke="none" />
  </>,
);
export const BackIcon = svg(<path d="m15 6-6 6 6 6" />);
export const ScissorsIcon = svg(
  <>
    <circle cx="6" cy="6" r="3" />
    <circle cx="6" cy="18" r="3" />
    <path d="M20 4 8.1 15.9M14.5 14.5 20 20M8.1 8.1 12 12" />
  </>,
);
export const FolderIcon = svg(<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />);
export const MusicIcon = svg(
  <>
    <path d="M9 18V5l11-2v13" />
    <circle cx="6" cy="18" r="3" />
    <circle cx="17" cy="16" r="3" />
  </>,
);
