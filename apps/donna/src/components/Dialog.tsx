import type { ComponentChildren } from "preact";
import { useEffect, useRef } from "preact/hooks";

interface DialogProps {
  open: boolean;
  onClose: () => void;
  label: string;
  /** "sheet" slides in from the right; "modal" is centred. */
  variant?: "modal" | "sheet";
  children: ComponentChildren;
}

/** A native <dialog> opened modally: focus trap, Esc and inert background. */
export function Dialog({ open, onClose, label, variant = "modal", children }: DialogProps) {
  const ref = useRef<HTMLDialogElement>(null);

  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (open && !dialog.open) dialog.showModal();
    if (!open && dialog.open) dialog.close();
  }, [open]);

  return (
    <dialog
      ref={ref}
      class={`dialog dialog-${variant}`}
      aria-label={label}
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      onClick={(event) => {
        // A click on the backdrop lands on the dialog element itself.
        if (event.target === ref.current) onClose();
      }}
    >
      {open ? children : null}
    </dialog>
  );
}
