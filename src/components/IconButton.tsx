import { forwardRef, type ButtonHTMLAttributes } from 'react';
import { Icon, type IconName } from './Icon';

interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  icon: IconName;
  /** Accessible name; also shown as the tooltip. */
  label: string;
  active?: boolean;
  small?: boolean;
  fire?: boolean;
  iconSize?: number;
}

export const IconButton = forwardRef<HTMLButtonElement, IconButtonProps>(function IconButton(
  { icon, label, active, small, fire, iconSize, className, ...rest },
  ref,
) {
  const classes = ['icon-button', small && 'is-small', active && 'is-active', fire && 'is-fire', className]
    .filter(Boolean)
    .join(' ');
  return (
    <button ref={ref} type="button" className={classes} aria-label={label} title={label} {...rest}>
      <Icon name={icon} size={iconSize ?? (small ? 16 : 19)} />
    </button>
  );
});
