import { useEffect, useRef, type CSSProperties, type ReactNode } from 'react';

interface MenuProps {
  onClose: () => void;
  children: ReactNode;
  label: string;
  style?: CSSProperties;
}

/** Small popover menu: closes on outside click or Escape, focuses its first item. */
export function Menu({ onClose, children, label, style }: MenuProps) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    ref.current?.querySelector<HTMLButtonElement>('button')?.focus();
    const onPointer = (e: PointerEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        e.stopPropagation();
        onClose();
      } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        const items = Array.from(ref.current?.querySelectorAll<HTMLButtonElement>('button') ?? []);
        const i = items.indexOf(document.activeElement as HTMLButtonElement);
        const next = items[(i + (e.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length];
        next?.focus();
        e.preventDefault();
      }
    };
    document.addEventListener('pointerdown', onPointer, true);
    document.addEventListener('keydown', onKey, true);
    return () => {
      document.removeEventListener('pointerdown', onPointer, true);
      document.removeEventListener('keydown', onKey, true);
    };
  }, [onClose]);

  return (
    <div ref={ref} className="menu" role="menu" aria-label={label} style={style}>
      {children}
    </div>
  );
}
