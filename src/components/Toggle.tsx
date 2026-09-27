interface ToggleProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  disabled?: boolean;
  id?: string;
}

/** Accessible switch (role="switch"). */
export function Toggle({ checked, onChange, label, disabled, id }: ToggleProps) {
  return (
    <button
      id={id}
      type="button"
      role="switch"
      className="toggle"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
    />
  );
}
