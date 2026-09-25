// The Sparkwell crystal: ice facets around a small ember core (Fire & Ice).
// Mirrors src-tauri/icons/source/icon.svg.

import { useId } from 'react';

export function LogoMark({ size = 26 }: { size?: number }) {
  const id = useId().replace(/:/g, '');
  return (
    <svg width={size} height={size} viewBox="0 0 64 64" aria-hidden="true" focusable="false" className="logo-mark">
      <defs>
        <linearGradient id={`${id}l`} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#f2fdff" />
          <stop offset="0.45" stopColor="#8eeaf6" />
          <stop offset="1" stopColor="#2aa9c7" />
        </linearGradient>
        <linearGradient id={`${id}m`} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#9ef0fa" />
          <stop offset="0.5" stopColor="#3fcfe2" />
          <stop offset="1" stopColor="#157f9e" />
        </linearGradient>
        <linearGradient id={`${id}d`} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#56d9ea" />
          <stop offset="1" stopColor="#0d5f7a" />
        </linearGradient>
        <radialGradient id={`${id}e`} cx="0.5" cy="0.5" r="0.5">
          <stop offset="0" stopColor="#ffe2a8" />
          <stop offset="0.45" stopColor="#f39a3d" stopOpacity="0.85" />
          <stop offset="1" stopColor="#f39a3d" stopOpacity="0" />
        </radialGradient>
      </defs>
      <path d="M18 15 L22.5 28 L32 60 L11 35 Z" fill={`url(#${id}d)`} />
      <path d="M46 15 L41.5 28 L32 60 L53 35 Z" fill={`url(#${id}d)`} />
      <path d="M32 3 L22.5 28 L32 60 Z" fill={`url(#${id}l)`} />
      <path d="M32 3 L41.5 28 L32 60 Z" fill={`url(#${id}m)`} />
      <circle cx="32" cy="41" r="5.5" fill={`url(#${id}e)`} />
    </svg>
  );
}
