import { useState } from 'react';
import { Icon } from '../../components/Icon';
import { IconButton } from '../../components/IconButton';
import { Menu } from '../../components/Menu';
import type { SparkSummary } from '../../services/types';

const MAX_TAGS = 4;

interface BestMatchCardProps {
  spark: SparkSummary;
  copied: boolean;
  onCopy: (spark: SparkSummary) => void;
  onToggleFavorite: (spark: SparkSummary) => void;
  onEdit: (spark: SparkSummary) => void;
  onDelete: (spark: SparkSummary) => void;
}

/** The single decisive recommendation. Copy Spark is its only primary action. */
export function BestMatchCard({ spark, copied, onCopy, onToggleFavorite, onEdit, onDelete }: BestMatchCardProps) {
  const [menuOpen, setMenuOpen] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const shownTags = spark.tags.slice(0, MAX_TAGS);
  const hiddenTags = spark.tags.length - shownTags.length;

  const closeMenu = () => {
    setMenuOpen(false);
    setConfirmDelete(false);
  };

  return (
    <article className="match" aria-labelledby={`match-title-${spark.id}`}>
      <div className="match-top">
        <span className="badge">
          <Icon name="star" size={14} strokeWidth={1.8} />
          Best Match
        </span>
        <div className="match-top-actions">
          <IconButton
            small
            fire
            icon={spark.favorite ? 'starFilled' : 'star'}
            label={spark.favorite ? 'Remove from Favorites' : 'Add to Favorites'}
            active={spark.favorite}
            aria-pressed={spark.favorite}
            onClick={() => onToggleFavorite(spark)}
          />
          <IconButton
            small
            icon="more"
            label="More actions"
            aria-haspopup="menu"
            aria-expanded={menuOpen}
            onClick={() => setMenuOpen((o) => !o)}
          />
          {menuOpen && (
            <Menu label="Spark actions" onClose={closeMenu} style={{ top: 32, right: 0 }}>
              <button
                type="button"
                role="menuitem"
                className="menu-item"
                onClick={() => {
                  closeMenu();
                  onEdit(spark);
                }}
              >
                <Icon name="edit" size={16} /> Edit Spark
              </button>
              <button
                type="button"
                role="menuitem"
                className="menu-item is-danger"
                onClick={() => {
                  if (!confirmDelete) {
                    setConfirmDelete(true);
                    return;
                  }
                  closeMenu();
                  onDelete(spark);
                }}
              >
                <Icon name="trash" size={16} /> {confirmDelete ? 'Click again to delete' : 'Delete Spark'}
              </button>
            </Menu>
          )}
        </div>
      </div>

      <h2 className="match-title selectable" id={`match-title-${spark.id}`}>
        {spark.title}
      </h2>
      {spark.summary && <p className="match-summary selectable">{spark.summary}</p>}

      {shownTags.length > 0 && (
        <div className="chips" aria-label="Tags">
          {shownTags.map((tag) => (
            <span key={tag} className="chip">
              {tag}
            </span>
          ))}
          {hiddenTags > 0 && <span className="chip is-more">+{hiddenTags}</span>}
        </div>
      )}

      <button
        type="button"
        className={`button is-fire is-large is-block copy-spark${copied ? ' is-done' : ''}`}
        onClick={() => onCopy(spark)}
        title="Copy the complete Spark (Ctrl+Enter)"
      >
        {copied ? (
          <>
            Copied <Icon name="check" size={18} strokeWidth={2} />
          </>
        ) : (
          <>
            Copy Spark <Icon name="clipboard" size={18} />
          </>
        )}
      </button>
    </article>
  );
}
