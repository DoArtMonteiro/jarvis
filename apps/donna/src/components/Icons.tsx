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
