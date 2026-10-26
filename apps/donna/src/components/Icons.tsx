import type { ComponentChildren } from "preact";

function Icon({ children, size = 18 }: { children: ComponentChildren; size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.8"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      {children}
    </svg>
  );
}

export const PlusIcon = () => (
  <Icon>
    <path d="M12 5v14M5 12h14" />
  </Icon>
);
export const SearchIcon = () => (
  <Icon>
    <circle cx="11" cy="11" r="6.5" />
    <path d="m16 16 4 4" />
  </Icon>
);
export const CloseIcon = () => (
  <Icon>
    <path d="M6 6l12 12M18 6 6 18" />
  </Icon>
);
export const TrashIcon = () => (
  <Icon>
    <path d="M4 7h16M10 11v6M14 11v6M6 7l1 12h10l1-12M9 7V4h6v3" />
  </Icon>
);
export const BellIcon = () => (
  <Icon>
    <path d="M6 16V11a6 6 0 0 1 12 0v5l1.5 2h-15zM10 20.5a2 2 0 0 0 4 0" />
  </Icon>
);
export const GearIcon = () => (
  <Icon>
    <circle cx="12" cy="12" r="3" />
    <path d="M12 3v2.5M12 18.5V21M3 12h2.5M18.5 12H21M5.6 5.6l1.8 1.8M16.6 16.6l1.8 1.8M18.4 5.6l-1.8 1.8M7.4 16.6l-1.8 1.8" />
  </Icon>
);
export const SendIcon = () => (
  <Icon>
    <path d="M5 12 19 5l-4 14-3-6z" />
  </Icon>
);
export const StopIcon = () => (
  <Icon>
    <rect x="7" y="7" width="10" height="10" rx="2" fill="currentColor" />
  </Icon>
);
export const RefreshIcon = () => (
  <Icon size={16}>
    <path d="M20 11a8 8 0 1 0-2.3 5.7M20 5v6h-6" />
  </Icon>
);
export const SidebarIcon = () => (
  <Icon>
    <rect x="4" y="5" width="16" height="14" rx="2" />
    <path d="M9.5 5v14" />
  </Icon>
);
export const ArrowLeftIcon = () => (
  <Icon>
    <path d="M19 12H5M11 6l-6 6 6 6" />
  </Icon>
);
export const HomeIcon = () => (
  <Icon>
    <path d="M4 11l8-7 8 7v9a1 1 0 0 1-1 1h-5v-6h-4v6H5a1 1 0 0 1-1-1z" />
  </Icon>
);
export const ChatIcon = () => (
  <Icon>
    <path d="M21 12a8 8 0 0 1-11.6 7.1L4 20l1-4.6A8 8 0 1 1 21 12z" />
  </Icon>
);
export const WorkIcon = () => (
  <Icon>
    <rect x="3" y="7" width="18" height="13" rx="1.5" />
    <path d="M8 7V4.5h8V7M3 12h18" />
  </Icon>
);
export const SystemIcon = () => (
  <Icon>
    <rect x="7" y="7" width="10" height="10" rx="1.5" />
    <path d="M10 3v4M14 3v4M10 17v4M14 17v4M3 10h4M3 14h4M17 10h4M17 14h4" />
  </Icon>
);
export const ShieldIcon = () => (
  <Icon>
    <path d="M12 3l8 3v6c0 4.5-3.2 8-8 9-4.8-1-8-4.5-8-9V6z" />
    <path d="m8.5 12 2.3 2.3L15.5 9.5" />
  </Icon>
);
export const LockIcon = () => (
  <Icon size={14}>
    <rect x="5" y="11" width="14" height="9" rx="1.5" />
    <path d="M8 11V8a4 4 0 0 1 8 0v3" />
  </Icon>
);
export const CheckIcon = () => (
  <Icon size={14}>
    <path d="m5 12.5 4.5 4.5L19 7.5" />
  </Icon>
);
export const ClockIcon = () => (
  <Icon size={14}>
    <circle cx="12" cy="12" r="8.5" />
    <path d="M12 7.5V12l3 2" />
  </Icon>
);
export const AlertIcon = () => (
  <Icon size={14}>
    <path d="M12 4 2.8 19.5h18.4z" />
    <path d="M12 10v4.5M12 17.2v.3" />
  </Icon>
);
export const ErrorIcon = () => (
  <Icon size={14}>
    <circle cx="12" cy="12" r="8.5" />
    <path d="m9 9 6 6M15 9l-6 6" />
  </Icon>
);
export const MinusIcon = () => (
  <Icon size={14}>
    <circle cx="12" cy="12" r="8.5" />
    <path d="M8.5 12h7" />
  </Icon>
);
export const DownloadIcon = () => (
  <Icon size={16}>
    <path d="M12 4v11M7 10.5l5 5 5-5M5 20h14" />
  </Icon>
);
export const HistoryIcon = () => (
  <Icon>
    <path d="M4 6h16M4 12h16M4 18h10" />
  </Icon>
);
export const ArrowRightIcon = () => (
  <Icon size={16}>
    <path d="M5 12h14M13 6l6 6-6 6" />
  </Icon>
);
