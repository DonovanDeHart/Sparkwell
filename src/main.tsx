import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import './styles/tokens.css';
import './styles/base.css';
import './components/controls.css';
import './app/shell.css';
import './features/search/search.css';
import './features/favorites/favorites.css';
import { App } from './app/App';

// Block the browser context menu and reload shortcuts in the desktop shell;
// text fields keep their native editing menu.
window.addEventListener('contextmenu', (e) => {
  const target = e.target as HTMLElement | null;
  if (!target?.closest('input, textarea, .selectable')) e.preventDefault();
});
window.addEventListener('keydown', (e) => {
  if (e.key === 'F5' || ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'r')) e.preventDefault();
});

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
