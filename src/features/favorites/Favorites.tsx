import { Icon } from '../../components/Icon';
import type { SparkSummary } from '../../services/types';
import { SparkRow } from '../sparks/SparkRow';

interface FavoritesProps {
  favorites: SparkSummary[] | null;
  copiedId: number | null;
  onCopy: (spark: SparkSummary) => void;
  onToggleFavorite: (spark: SparkSummary) => void;
}

/** Favorites replace Collections in the MVP: one click copies, no navigation. */
export function Favorites({ favorites, copiedId, onCopy, onToggleFavorite }: FavoritesProps) {
  return (
    <section className="favorites" aria-labelledby="favorites-title">
      <h2 className="section-title" id="favorites-title">
        Favorites
        {favorites && favorites.length > 0 && <span className="section-count">{favorites.length}</span>}
      </h2>
      {favorites === null ? null : favorites.length === 0 ? (
        <p className="favorites-empty">
          <Icon name="star" size={16} />
          Star a Spark to keep it here for one-click copying.
        </p>
      ) : (
        <ul className="spark-rows">
          {favorites.map((spark) => (
            <SparkRow
              key={spark.id}
              spark={spark}
              copied={copiedId === spark.id}
              onCopy={onCopy}
              onToggleFavorite={onToggleFavorite}
            />
          ))}
        </ul>
      )}
    </section>
  );
}
